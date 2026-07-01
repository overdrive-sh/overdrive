# Root-Cause Analysis — in-flight churn "fail fast" never surfaces (S-DBN-CHURN)

**Analyst:** Rex (Toyota 5-Whys RCA) · **Date:** 2026-07-01 · **Kernel:** dev-Lima `7.0.0-22-generic`
**Subject test:** `in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend`
(scenario **S-DBN-CHURN**) in
`crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`
**Scope:** why the held in-flight read never returns error/EOF within the window.
Investigation only — no production change, no test change, no commit.

---

## PROBLEM STATEMENT (scoped)

A client holding an OPEN in-flight plaintext connection through frontend `F` to
backend `B1`, when `B1` is cycled mid-connection via the production
`overdrive workload restart` verb, does NOT get a prompt reset/error/EOF. The
in-flight read censors at `CHURN_BOUND` (30 s) — it never returns error/EOF
within the window. The AC requires the connection to **fail fast** (reset/error/
EOF) bounded by `TCP_USER_TIMEOUT`, surfaced through the per-connection pump task
+ `TCP_USER_TIMEOUT`/keepalive on the worker proxy legs, with **NO `sock_destroy`**.

---

## EVIDENCE INDEX (all pasted, none narrated)

### E0 — anchor reproduction (the real AT)

```
$ cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
    --features integration-tests \
    -E 'test(in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend)' --no-capture
...
[02-02] uname -r = 7.0.0-22-generic (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)
[02-02] getent ahostsv4 server.svc.overdrive.local in ovd-ns-0001 -> [10.98.0.1] (code Some(0))
[03-01] S-DBN-CHURN: resolved F = 10.98.0.1, alloc B1 = alloc-server-0

thread '...in_flight_connection_fails_fast_on_backend_churn...' panicked at ...:1972:5:
S-DBN-CHURN: the in-flight connection must fail FAST ... Observed elapsed: 30.690446761s (bound 30s).
...
     TIMEOUT [ 120.067s] (1/1) ...in_flight_connection_fails_fast_on_backend_churn...
```

Two facts: (a) the loop exits on the `CHURN_BOUND` guard (30.690 s ≳ 30 s), not
on a read returning error/EOF; (b) after the panic the process lingers to
nextest's 120 s SIGKILL — a leaked/undrained pump task.

### E1 — no-restart hold probe (the decisive population half; §5 population diff)

Temporary probe (added, run, reverted — tree restored). Hold the in-flight
connection open with **NO restart at all**.

- **Hypothesis:** the server closes each connection after ONE response
  (`finally: c.close()`), producing a CLEAN FIN on leg-B that the return decrypt
  pump classifies `PumpExit::Graceful` and does NOT propagate to leg-F — so the
  held read hangs even without any restart.
- **Predicted:** first round-trip completes (RESPONSE seen); held read censors at
  `PROBE_BOUND` (12 s).
- **Falsification:** the held read returns EOF/error promptly with no restart ⇒
  the hang is restart-specific (H1 drain-deadlock live).

```
[PROBE] resolved F = 10.98.0.1
[PROBE] first round-trip: got 80 of 80 RESPONSE bytes (completed=true)
[PROBE] held read CENSORED at 12.245258616s (no EOF/error surfaced)
[PROBE] RESULT first_rt_completed=true held_read_returned=false held_elapsed=12.245324796s
test ...rca_probe_hold_in_flight_no_restart_surfaces_death ... ok
```

`first_rt_completed=true` ⇒ the connection WAS genuinely live THROUGH B1
(byte-exact 80/80 RESPONSE). `held_read_returned=false` at 12.245 s **with no
restart** ⇒ the hang is NOT restart-induced.

### E2 — churn-side probe (the other population half; B1 termination timing)

Temporary probe (added, run, reverted). Hold the connection, fire the REAL
restart, concurrently sample B1's `alloc_status`.

- **Hypothesis (H1 refutation):** B1 reaches Terminated within a few seconds of
  the restart even while the connection is held — the drain does NOT wait on the
  client-held connection.
- **Predicted:** B1 Terminated within ~10 s; the held read STILL censors.
- **Falsification:** B1 never reaches Terminated within the window ⇒ H1
  drain-deadlock live.

```
[PROBE2] resolved F = 10.98.0.1, alloc B1 = alloc-server-0
[PROBE] first round-trip: got 80 of 80 RESPONSE bytes (completed=true)
[PROBE2] restart accepted=true
[PROBE2] B1 (alloc-server-0) reached Terminated 206.898864ms after restart (connection STILL held)
[PROBE] held read CENSORED at 12.26111149s (no EOF/error surfaced)
[PROBE2] HELD-READ first_rt_completed=true held_read_returned=false held_elapsed=12.261168064s
test ...rca_probe_churn_b1_terminates_while_connection_held ... ok
```

**B1 reaches Terminated 206.9 ms after the restart — while the connection is
still held.** The drain does NOT wait on the client-held connection. Yet the held
read STILL censors at 12.26 s. B1 is long dead; its death simply never reaches
leg-F.

### E3 — the server closes each connection after one response (source read)

`server_service_spec` (`dns_responder_walking_skeleton.rs:749-765`):

```python
while True:
    c, _ = s.accept()
    try:
        _ = c.recv(4096)
        c.sendall(b'{response_py}')
    except Exception:
        pass
    finally:
        c.close()          # <-- per-connection clean close after ONE response
```

Each accepted connection gets exactly one response, then `c.close()` — a clean
FIN on the server's socket, independent of any restart.

### E4 — the return decrypt pump treats a clean FIN as `Graceful` (source read)

`crates/overdrive-dataplane/src/mtls/splice.rs::run_decrypt_pump` (the return
pump `splice(legB → legF)`, the sole observer of leg-B death while the workload
is idle):

```rust
if pfd.revents & libc::POLLIN == 0 {
    if pfd.revents & libc::POLLERR != 0 { break PumpExit::TransportDeath; }
    if pfd.revents & libc::POLLHUP != 0 { break PumpExit::Graceful; }  // clean FIN → Graceful
    ...
}
...
if n_in == 0 { break PumpExit::Graceful; }  // clean EOF on the source → Graceful
```

`mark_exited` (same file) fires self-teardown ONLY for `TransportDeath`:

```rust
fn mark_exited(state: &PumpState, exit: PumpExit) {
    state.running.store(false, Ordering::SeqCst);
    if exit == PumpExit::TransportDeath { state.fire_self_teardown_if_unexpected(); }
}
```

and the design intent is pinned by the unit test
`graceful_eof_exit_does_not_fire_self_teardown` ("a clean EOF half-close must NOT
self-tear-down (sibling direction may be live)"). Consequence: on a clean FIN,
`fire_self_teardown_if_unexpected` is NEVER called → `reclaim_connection` never
runs → leg-F is never closed.

### E5 — leg-F is closed ONLY by `reclaim_connection` (source read)

`crates/overdrive-dataplane/src/mtls/mod.rs::reclaim_connection` (shared by the
deliberate `teardown` and the (B) self-teardown reaper) is the only site that
`drop(state.legs)` (closes leg-F). It is reached from (a) `teardown(handle)` —
which the worker only calls on `stop_alloc` of the *client* alloc, not B1's — or
(b) the (B) self-teardown trigger, which E4 shows never fires on `Graceful`.

### E6 — `TCP_USER_TIMEOUT` reaps unacked retransmission, not a clean close (source read)

`crates/overdrive-dataplane/src/mtls/mod.rs::arm_transport_death_timeouts`
(rustdoc): "`TCP_USER_TIMEOUT` (the max time the kernel keeps retransmitting
**unacked data — or unacked keepalive probes** — before failing the socket
`ETIMEDOUT`)". A connection the peer closed **cleanly** (FIN, fully ACKed) has
nothing unacked outstanding — `TCP_USER_TIMEOUT` has nothing to time against, so
it never fires. It reaps a peer that *vanished* (silent, unacked), not one that
*closed*.

### E7 — the v1 stall watchdog is a RESERVED, not-tick-driven predicate (source read)

`crates/overdrive-dataplane/src/mtls/supervision.rs` (module docstring):
"`derive_liveness` + `PumpLiveness::Stalled` are RETAINED as ... the RESERVED
predicate for the deferred kernel-invisible progress-stall watchdog (#232 ...).
They are NOT driven by a tick in v1." So there is no third mechanism that could
surface the death; v1 has exactly (B) self-teardown-on-`TransportDeath` and (C)
kernel `TCP_USER_TIMEOUT`. E4 disqualifies (B) for a clean close; E6 disqualifies
(C).

### E8 — the AC premise (source read)

`docs/feature/dial-by-name-responder/distill/test-scenarios.md:1349-1351`
(S-DBN-CHURN spec): "NO sock_destroy is used (**backend death surfaces through
the per-connection pump task + TCP_USER_TIMEOUT/keepalive** — the
terminating-proxy posture)". This is the premise E4/E6 falsify for a
clean-close backend.

---

## THE 5-WHYS (multi-causal, branched)

```
PROBLEM: the held in-flight read never returns reset/error/EOF within CHURN_BOUND (censors at ~30 s).

WHY 1 (symptom): the client's leg-F socket is never closed and never RST'd, so its blocking read
        never surfaces EOF/error.
        [Evidence: E0 loop exits on the CHURN_BOUND guard; E1/E2 held_read_returned=false; E5 only
         reclaim_connection closes leg-F.]

  WHY 2A (the death arrives as a CLEAN half-close, not a transport death):
          the backend socket to leg-B is closed CLEANLY (FIN), not reset (RST) or vanished.
          [Evidence: E3 the server does finally: c.close() after one response (a clean FIN);
           E1 the hang happens with NO restart at all, driven purely by that per-connection close;
           E2 even under the real restart, B1 exits on SIGTERM (Driver::stop sends SIGTERM first,
           5 s grace — driver.rs:620) → Python exits normally → kernel FINs its sockets, and the
           per-connection socket was already FIN'd by c.close() before the restart.]

    WHY 3A (the return pump classifies a clean FIN as PumpExit::Graceful and does not propagate it):
            run_decrypt_pump breaks Graceful on POLLHUP-with-no-POLLIN or n_in==0; mark_exited fires
            the (B) self-teardown ONLY for PumpExit::TransportDeath.
            [Evidence: E4 splice.rs run_decrypt_pump + mark_exited; the unit test
             graceful_eof_exit_does_not_fire_self_teardown pins this as intent.]

      WHY 4A (the design deliberately does NOT tear a connection down on a clean half-close):
              a clean EOF on one direction must not nuke a connection whose sibling direction may
              still be live (bidirectional half-close correctness).
              [Evidence: E4 register()/mark_exited rustdoc: "a clean half-close MUST NOT nuke a
               connection whose request path is still live" — the PumpExit::Graceful gate.]

        WHY 5A (ROOT CAUSE A — DESIGN/AC GAP): the AC assumes backend death always presents as a
              TransportDeath the pump+TCP_USER_TIMEOUT can reap, but a backend that closes CLEANLY
              (the common, benign case — a graceful exit, an HTTP/1.0-style one-response-then-close
              server, a SIGTERM'd process) presents as a Graceful half-close, which by design does
              NOT close the opposing (client-facing) leg. There is no v1 mechanism that turns
              "backend closed cleanly" into "close the client leg too."
              [Evidence: E7 the stall watchdog is reserved/not tick-driven (#232); E4 Graceful is
               deliberately non-reclaiming; E8 the AC premise.]

  WHY 2B (TCP_USER_TIMEOUT cannot reap a cleanly-closed connection):
          the kernel TCP_USER_TIMEOUT reaps unacked retransmission / unacked keepalive probes; a
          peer that FIN-closed has nothing unacked outstanding.
          [Evidence: E6 arm_transport_death_timeouts rustdoc; E1 the hang persists 12 s with the
           timeouts armed and no restart.]

    WHY 3B (the (C) kernel mechanism is the wrong tool for THIS death mode):
            TCP_USER_TIMEOUT/keepalive detects a peer that VANISHED (silent, no FIN/RST), not one
            that CLOSED. Keepalive probes only start after TCP_KEEPIDLE (10 s) of idle and only on
            an otherwise-established socket — a FIN-closed socket is not "idle established," it is
            half-closed, and no probe is sent.
            [Evidence: E6; KEEPALIVE_IDLE_SECS=10 / arm_transport_death_timeouts in mod.rs.]

      WHY 4B (same design blind spot as 4A, reached via the kernel path):
              (C) is scoped to the transport-death class only; the clean-close class was not in the
              (C) threat model.
              [Evidence: E6 "a peer that vanished without a FIN/RST".]

        WHY 5B (ROOT CAUSE A, same fundamental): a clean-close backend is invisible to BOTH v1
              surfacing mechanisms — (B) fires only on TransportDeath, (C) reaps only unacked
              transport death. They collectively do not cover "backend closed cleanly." (Converges
              on Root Cause A.)

  WHY 2C (the test client can only observe leg-F, and the death never reaches leg-F):
          churn_in_flight_read loops reads on the client TcpStream waiting for EOF/error; the only
          leg it can observe is leg-F, which E5 shows is never closed on this path.
          [Evidence: E1/E2 held_read_returned=false; E5.]

    WHY 3C (the TEST MODEL expects the RESTART to be what surfaces the death, but the connection is
            already half-closed the instant the first response lands):
            the test holds the connection AFTER a first read (let _ = tcp.read), then times the
            SECOND read against the churn — but the server's per-connection c.close() already
            half-closed the backend leg before the churn window opens, so there is nothing left for
            the churn to surface, and the surviving half (leg-F) is a Graceful casualty that never
            closes.
            [Evidence: E1 the identical hang with NO restart — the restart is not the trigger;
             E3 the per-connection close; the test's churn_in_flight_read at :2063-2120 discards the
             first read and never asserts a live full-duplex hold.]

      WHY 4C (the test conflates "backend instance replaced" with "in-flight connection killed"):
              the AC's mental model is "cycle B1 → in-flight connection to B1 dies fast." But on the
              actual datapath the in-flight connection is a chain of proxied legs; B1's death (or its
              per-connection close) reaches the agent's legs, not the client's leg-F, and the agent
              does not translate a Graceful backend close into a client-leg close.
              [Evidence: E4/E5 the leg topology + Graceful non-reclaim; E2 B1 dies in 207 ms yet
               leg-F persists 12 s.]

        WHY 5C (ROOT CAUSE C — TEST-MODEL ERROR): the test uses a server that closes each connection
              after one response, so the "in-flight connection" is single-shot: after the first
              round-trip the backend leg is already cleanly half-closed. The test then attributes the
              subsequent hang to "the restart didn't surface the death," when in fact NO restart is
              needed to reproduce the hang (E1). The test cannot distinguish "the datapath failed to
              propagate a backend death" from "the server closed the connection normally and the
              proxy correctly kept the (now half-closed) connection open."
              [Evidence: E1 no-restart reproduction; E3 the single-shot server.]
```

### Refuted hypotheses (with the evidence that settled each)

- **H1 — drain-deadlock on the churn path (B1 never reaches Terminated because
  the drain waits on the client-held connection): REFUTED.**
  E2: B1 (`alloc-server-0`) reached **Terminated 206.9 ms** after the restart
  *while the connection was still held*. The `StopAllocation`/drain does not wait
  on the client-held connection. Prediction of H1 ("B1 never Terminated within
  the window") falsified.

- **H3 — test-model error where the first round-trip never completed (the "in-
  flight connection" was never live through B1): REFUTED.**
  E1/E2: `first_rt_completed=true`, **80/80** RESPONSE bytes byte-exact. The
  connection WAS live through B1; the first plaintext round-trip completed.

- **H2 (as literally stated — "teardown-propagation gap: B1 dies but the pump
  chain doesn't propagate the close to leg-F"): CONFIRMED as the *mechanism*, but
  the root cause is NOT a bug in the propagation — it is that the close is
  **Graceful by design** and the design has no path from "backend closed cleanly"
  to "close the client leg."** The propagation "gap" is intentional half-close
  correctness (WHY 4A), so the finding is a design/AC gap (Root Cause A), not a
  broken pump.

---

## VALIDATED ROOT CAUSES

**Root Cause A (DESIGN / AC gap) — the dominant cause.**
The v1 transparent-mTLS proxy has exactly two liveness-surfacing mechanisms: (B)
per-connection self-teardown fired ONLY on `PumpExit::TransportDeath`, and (C)
kernel `TCP_USER_TIMEOUT`/keepalive that reaps only unacked transport death. A
backend that closes its connection **cleanly** (FIN) — which includes the S-DBN-
CHURN server's per-connection `c.close()`, and a `SIGTERM`-terminated backend
that exits normally and lets the kernel FIN its sockets — presents as a
`PumpExit::Graceful` half-close, which is deliberately non-reclaiming (WHY 4A,
bidirectional half-close correctness), and has nothing unacked for `TCP_USER_
TIMEOUT` to reap (WHY 3B). So the backend's clean close never propagates to close
the client-facing leg-F. **The AC's premise — "backend death surfaces through the
per-connection pump task + `TCP_USER_TIMEOUT`/keepalive" — is unsatisfiable for
the clean-close case with the excluded-`sock_destroy` toolset.**

**Root Cause C (TEST-MODEL error) — compounding, and the reason the AT can never
go green as written.**
The test's server closes each connection after one response (`finally:
c.close()`), so the "in-flight connection" is single-shot: the backend leg is
already cleanly half-closed the instant the first round-trip lands, *before the
churn window even opens*. E1 proves the identical hang reproduces **with no
restart at all** — the restart is not the trigger. The test therefore cannot
isolate "the restart surfaced the backend death" from "the server closed the
connection normally," and it attributes a benign, correct half-close outcome to a
churn-fail-fast defect.

**Cross-validation (backwards chain).** If Root Cause A holds, then a
cleanly-closing backend leaves the client leg open indefinitely → the held read
hangs → censors at the bound. Observed: E0/E1/E2 all censor. ✔ If Root Cause C
holds, the hang must reproduce without a restart. Observed: E1 censors at 12.2 s
with no restart. ✔ The two root causes are consistent, not contradictory: C is
*why the test exercises* the clean-close path, A is *why the datapath does not
surface it*. Together they explain every symptom (the censor, the 207 ms-B1-death
with persistent leg-F, the 120 s process linger from the undrained pump).

**One residual note (not a separate root cause).** The 120 s process-linger after
the panic (E0) is the same Graceful non-reclaim: on `skeleton.shutdown()` the
client alloc's `stop_alloc` seals+drains, but any connection whose pumps exited
Graceful without reclaim leaves a `ConnState` entry the shutdown path does not
force-close synchronously — consistent with the undrained-pump linger. It is a
downstream consequence of Root Cause A, not independent.

---

## RECOMMENDED FIXES (each mapped to a root cause, classified)

### For Root Cause C — TEST-MODEL fix (necessary regardless of A)

**T1 (test-model-fix).** The server must model a **long-lived, full-duplex**
backend that does NOT close after one response, so the "in-flight connection" is
genuinely still open when the churn fires. Concretely, in
`server_service_spec` (`dns_responder_walking_skeleton.rs:749`) the per-accept
handler must keep the connection open (loop reading, or block) rather than
`c.close()` in a `finally` after one `sendall`. Only then does "cycle B1" become
the actual cause of the connection's death, and only then can the AT distinguish
a fail-fast defect from a normal close. **Without T1 the AT is unfalsifiable — it
hangs identically with no churn (E1).**

**T2 (test-model-fix, dependent on the A decision).** Once T1 makes the backend
long-lived, `churn_in_flight_read` should assert the first full round-trip
completed byte-exact **before** holding (promote the current `let _ =
tcp.read(...)` at :2092 to an explicit `got == RESPONSE` gate — the same shape the
working `dial()` helper uses at :883), so a future regression where the hold
socket was never plumbed through B1 (H3-shaped) is caught rather than masked.

### For Root Cause A — the design/AC reconciliation (the landability gate)

The AC as written ("backend death surfaces through the pump task +
`TCP_USER_TIMEOUT`/keepalive; NO `sock_destroy`") is **not satisfiable for a
backend that closes cleanly** — `TCP_USER_TIMEOUT` reaps only unacked transport
death (E6), and the (B) trigger is deliberately Graceful-exempt (E4). Two
mutually exclusive resolutions; **both are design decisions for the user/architect,
not something to invent here.**

**A1 (production-fix — propagate a clean backend half-close to the client leg).**
Make a clean EOF on the *request-carrying* leg propagate a half-close (or full
close) to the opposing leg, instead of leaving the client leg open forever.
Surface: `crates/overdrive-dataplane/src/mtls/splice.rs` — the `PumpExit::Graceful`
handling in `run_decrypt_pump` / `run_encrypt_pump`, and the reclaim decision in
`crates/overdrive-dataplane/src/mtls/mod.rs::mark_exited` /
`self_teardown_trigger` / `reclaim_connection`. **This is genuinely design-
sensitive** and MUST be pinned by the architect before a crafter touches it: a
naive "fire self-teardown on Graceful too" would break the half-close
correctness the `PumpExit::Graceful` gate exists to protect (WHY 4A) — a backend
that half-closes its *write* side while still reading (a legitimate
`shutdown(SHUT_WR)`) would be wrongly nuked. The correct shape is likely a
`shutdown(leg_f, SHUT_WR)` **half-close forward** on a backend read-EOF (mirroring
the peer's FIN to the client) rather than a full connection reclaim — but the
exact semantics (half-close-forward vs. full-close, and how the sibling
direction's own EOF then completes the reclaim) is an ADR-0070/ADR-0069-level
decision. **Do not invent it; surface the gap.**

**A2 (AC/design-gap — narrow the AC to the transport-death class the toolset can
actually surface).** If the platform intent is that a *cleanly-closing* backend
is a benign event the client observes through its own normal read-EOF (i.e. the
proxy is transparent and a FIN should flow end-to-end anyway — which requires A1
to actually deliver that FIN), then the AC's "fail fast bounded by
`TCP_USER_TIMEOUT`" clause applies only to the **transport-death** class (a
backend that *vanishes* — SIGKILL with unflushed data → RST, or a silent host
drop). The AC would then be reworded to cycle B1 in a way that produces a
**transport death** (e.g. assert on a backend killed hard enough to RST, or a
netem blackhole of the backend leg so keepalive/`TCP_USER_TIMEOUT` fires), and
the clean-close path becomes a *separate* "FIN flows end-to-end" AC. This is an
`nw-solution-architect` decision against ADR-0070 (the (C)+(B) supervision
model) and the S-DBN-CHURN scenario spec
(`docs/feature/dial-by-name-responder/distill/test-scenarios.md:1330`).

**`sock_destroy` remains excluded (#61).** Neither A1 nor A2 needs it; A1 is a
userspace-leg half-close/close, A2 relies on the existing kernel `TCP_USER_
TIMEOUT`/keepalive for the transport-death class. The AC's "NO `sock_destroy`"
constraint is preserved under both. (Had the ONLY way to surface the clean-close
death been `sock_destroy`, that would itself be an A2-shaped AC gap — but it is
not: A1's userspace half-close is available.)

### Early detection (prevention)

- **D1.** Add a Tier-1/Tier-3 assertion that a backend read-EOF results in a
  bounded client-leg EOF (once A1 lands) — the direct regression guard for Root
  Cause A. Until A1/A2 is decided, keep S-DBN-CHURN `#[ignore]`'d (its current
  state) rather than shipping a green-over-broken suite.
- **D2.** A DST/unit test over `mark_exited` that pins the *chosen* clean-close
  semantics (whichever of A1/A2 lands), so the half-close-correctness invariant
  and the new clean-close-propagation invariant cannot silently diverge.

---

## LANDABILITY VERDICT (for the orchestrator)

**03-01 is NOT landable as a bounded change.** It is blocked on a design/AC
decision (Root Cause A) that is out of scope for a step-level crafter:

- The **test-model fix (T1/T2)** is necessary but *not sufficient* — even with a
  long-lived full-duplex backend, a clean `SIGTERM` shutdown of B1 still presents
  as a Graceful FIN that today's datapath does not propagate to leg-F, so the AT
  would still hang unless A1 lands or the AC is narrowed (A2).
- The **production fix (A1)** touches ADR-0070/ADR-0069 supervision semantics
  (half-close-forward vs. full reclaim) and MUST be pinned by the architect first
  — a crafter inventing the shape would either break half-close correctness or
  diverge from the design (CLAUDE.md "Implement to the design — never invent API
  surface").
- The **AC narrowing (A2)** is likewise an architect decision against the
  S-DBN-CHURN scenario spec.

**Recommendation to relay to the user:** treat the clean-close propagation (A1) —
OR the AC re-scoping (A2) — as a **separate design slice** (`nw-solution-architect`
against ADR-0070), and land the test-model fix (T1/T2) as part of whichever
resolution is chosen. Keep S-DBN-CHURN `#[ignore]`'d (citing the design gap, not
the already-shipped restart verb) until that slice lands. The restart verb itself
works correctly (E2: B1 Terminated in 207 ms; the sibling S-DBN-WS-STABLE is
green) — the churn AT's blocker is the clean-close surfacing gap, which the
restart-verb swap merely *revealed*, it did not cause.

**Issue recommendation (for the user to approve — NOT created here):** a tracking
issue for "transparent-mTLS: a cleanly-closing backend (FIN) does not propagate a
close to the client-facing leg (leg-F); clean-close is invisible to both (B)
self-teardown and (C) `TCP_USER_TIMEOUT`" — the A1/A2 decision and the S-DBN-CHURN
un-ignore would hang off it. Do not create it without the user's go-ahead.

---

## Working-tree note

All probing was done with two temporary `#[tokio::test]` probes added to
`dns_responder_walking_skeleton.rs`, run under Lima, then removed. The file is
restored to its as-found state (the pre-existing session-start modification — the
un-ignored AT + production restart-verb swap — is preserved; `CHURN_BOUND` = 30 s;
uncommitted). No production source was modified. Leaked Lima cgroup/netns/veth
state from the runs was swept.
