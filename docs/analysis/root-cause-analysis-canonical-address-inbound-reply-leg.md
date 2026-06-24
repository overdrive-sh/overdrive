# RCA — canonical-address inbound mTLS **reply leg** "no byte-exact RESPONSE" (S-WS keystone, GH #241)

- **Subject test:** `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs::workload_reached_at_canonical_address_terminates_mtls_end_to_end`
- **Feature / step:** `canonical-workload-address-inbound-tproxy` (GH #241), step 03-02 (S-WS keystone).
- **Predecessor RCA:** `docs/analysis/root-cause-analysis-canonical-address-inbound-roundtrip-hang.md` (the routing/teardown convergence bug, fixed in commit `f034f38f` — `FinalizeFailed{Stable}` no longer tears down a live Running alloc's netns). With that fixed, the dial now connects, the handshake completes, and the request reaches the server — the keystone advanced to a NEW wall on the reply leg.
- **Kernel:** `uname -r = 7.0.0-22-generic` (dev Lima; merge gate is the pinned-6.18 Tier-3 matrix, ADR-0068).
- **Verdict:** **(T) TEST-COMPOSITION GAP — fixable INSIDE step 03-02.** The entire production inbound mTLS pipe — leg-C terminate → leg-S splice → server receipt → server reply → leg-S read → leg-C kTLS-TX encrypt → encrypted record on the client wire → client decrypt — **works byte-exact, proven on the wire.** The keystone's server is an **echo** server (`c.sendall(buf)` — replies with the bytes it received), but the assertion compares the client's received bytes against a *distinct* `RESPONSE` constant. The client receives the echoed **REQUEST** (81 bytes) byte-exact; the test expects the **RESPONSE** (87 bytes); `got != RESPONSE`. **This is NOT a production reply-leg defect.**
- **Investigation discipline:** every Lima command host-`timeout`-wrapped (`timeout 420`/`360`); Bash tool timeout 300–450 s; VM scrubbed (cgroups/netns/veths/nft/bpffs) after every run and left clean; no `| tail`/`| head` on the test run itself; never `--no-run`; diagnostics written to guest files and read back host-timeout-wrapped; tcpdump killed by argv-specific `pkill -f 'tcpdump -i '` (never a pattern matching the scrub shell). Throwaway test-side instrumentation was added for evidence and **fully reverted** (restored from backup; `git diff` clean). No production source was modified.

---

## 1. ⚠️ BASELINE CROSS-CHECK — is the encrypted inbound reply pipe proven-working?

The brief flagged that a prior agent labelled this a "production reply-leg defect" *by analogy*. The evidence disproves that analogy on two independent axes.

### 1.1 increment-i (`docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md`)

increment-i proved the inbound **request** half (C→S) byte-exact on kernel 7.0 — TPROXY intercept + orig-dst recovery + server-side mTLS + kTLS-RX decrypt + agent-light splice-to-S — and the fail-closed authn boundary. **But it explicitly did NOT prove the reply leg.** Its own § "What was NOT tested":

> **Bidirectional steady-state** — only C→S (request) was driven. The S→C **response leg** (re-encrypt S's plaintext reply back onto the client leg's kTLS-TX) **was not exercised** … composing it into this server shape is **unproven**.

So the "increment-i proved the composed inbound flow end-to-end" framing is **partially overstated for the reply leg** — increment-i is *not* the baseline that proves S→C. (It IS the baseline for the request leg, which this RCA's live evidence also confirms works.)

### 1.2 The OLD composed bidirectional test (`cb7d8d09`) — the REAL reply-leg baseline

`composed_bidirectional_mtls_completes_no_rst_with_tls13_wire_capture`
(`crates/overdrive-worker/tests/integration/bidirectional_walking_skeleton.rs` at commit `cb7d8d09`, before this feature gutted the synthetic-virt apparatus) **DID prove the inbound reply leg byte-exact, and was GREEN.** It drove the SAME production enforcement path the keystone uses (`adapter.enforce(inbound_conn)` → the production `establish` in `crates/overdrive-dataplane/src/mtls/inbound.rs`, which starts BOTH the C→S decrypt pump AND the S→C encrypt pump). Its oracle asserted the reply direction explicitly:

```rust
// cb7d8d09 bidirectional_walking_skeleton.rs ~:1693
assert!(
    client_result.received_response_byte_exact,
    "O3 inbound: the client must read S's response byte-exact back over leg-C's kTLS"
);
// ... and the wire oracle:
assert!(inbound_scan.records_from_wire_port > 0,
    "O2 inbound: the response direction (from the virt) must carry 0x17 records");
```

**Crucially, the OLD GREEN server replied with a DISTINCT constant, not an echo** (`bidirectional_walking_skeleton.rs` @ `cb7d8d09` `inbound_server_run`):

```rust
let request_ok = got == INBOUND_REQUEST;
let _ = tcp.write_all(INBOUND_RESPONSE).and_then(|()| tcp.flush());  // <-- DISTINCT RESPONSE
```

with `INBOUND_REQUEST` and `INBOUND_RESPONSE` two different byte strings. So the client's `got == INBOUND_RESPONSE` could only be true because S sent the *distinct* RESPONSE. **The proven-working reply pipe was always driven by a server that sends a RESPONSE distinct from the REQUEST.**

**Answer to the brief's question: YES, the encrypted inbound reply pipe is proven-working in the baseline** (the OLD `cb7d8d09` composed test, GREEN, with `records_from_wire_port > 0` and `received_response_byte_exact` over the SAME production `establish` path). increment-i is NOT that baseline (it left the reply leg untested), so any "production reply-leg defect" claim resting on increment-i alone is the under-selling trap.

---

## 2. POPULATION-DIFF — keystone reply path vs the proven-working baseline (debugging.md §5)

Same production reply path (`mtls/inbound.rs::establish` → `splice.rs::run_encrypt_pump`). The differences:

| Axis | OLD GREEN baseline (`cb7d8d09`) | Keystone (fails the assert) | Affects the reply? |
|---|---|---|---|
| **Server reply content** | reads `INBOUND_REQUEST`, writes **distinct `INBOUND_RESPONSE`** (`tcp.write_all(INBOUND_RESPONSE)`) | **echoes** the request (`c.sendall(buf)`) — replies with the REQUEST bytes | **YES — THIS IS THE DIFF** |
| Assertion | `got == INBOUND_RESPONSE` (server sent RESPONSE) | `got == RESPONSE` (server sent REQUEST) → mismatch | YES (consequence of the above) |
| Server S bind | `127.0.0.91:18811` on host `lo` (sibling of agent) | `0.0.0.0:18941` **inside netns `ovd-ns-0000`** | No (request reaches S either way) |
| leg-S dial target | `127.0.0.91:18811` (loopback) | `10.99.0.2:18941` (in-netns veth addr, crosses `ovd-hv-0000`) | No (proven below: reply traverses it fine) |
| Client source | host `lo` | netns `ovd-ks-cli-ns` (`10.99.200.2`), asymmetric path | No (proven below: reply reaches the client) |
| Composition | direct `accept_inbound_leg` + `adapter.enforce` in test | real `run_server` → accept_loop → `enforce` | No (same `establish` runs) |

**The diff is the diagnosis** (debugging.md §5): the ONE axis that touches the reply *content* is the server. The OLD GREEN server replied with a constant distinct from the request; the keystone server echoes the request. Every other axis (netns bind, veth-crossing leg-S, asymmetric client path, real-`run_server` composition) was ruled OUT below by direct wire evidence — they all carry the reply correctly.

---

## 3. THE FAILURE SHAPE (debugging.md §2 — the assert names the layer that gave up, not the mechanism)

The panic fires at `:841` — the SECOND assertion (`received_response_byte_exact`), NOT the first (`!observed_rst`):

```
[03-02] uname -r = 7.0.0-22-generic (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)
[03-02] server canonical workload_addr = 10.99.0.2:18941
thread '…workload_reached_at_canonical_address_terminates_mtls_end_to_end' panicked at
  …canonical_address_inbound_walking_skeleton.rs:841:5:
S-WS: the client must read the server's reply byte-exact back over the production leg-C kTLS …
```

`observed_rst == false` (no RST; write succeeded), `received_response_byte_exact == false` (read loop returned without the expected RESPONSE bytes). The 120 s nextest reap after the panic is the SECONDARY teardown-blocks-after-panic symptom (the panic unwinds before `keystone.shutdown()`, so `Keystone::drop` blocks) — cosmetic, lower priority, NOT the subject of this RCA.

---

## 4. UPSTREAM-WALK — the producer ran and the reply is on the wire (debugging.md §11)

An absent reply at the *consumer* (the client's read loop) is a downstream symptom — walk one step upstream to the producer. Three concentric wire/log captures (test-side instrumentation, since reverted) localise the loss layer by layer.

### 4.1 Server-side (the producer): the request reaches S, S replies — to its echo socket

Instrumented echo log (`/tmp/ks-server.log`), real output:

```
… LISTENING 0.0.0.0:18941
… ACCEPT from ('10.99.0.1', 42920) local ('10.99.0.2', 18941)
… RECV 81 bytes: b'OVERDRIVE_CANONICAL_ADDR_REQUEST_client_to_serve'
… SENDALL 81 bytes done
… CLOSED
```

- `ACCEPT from ('10.99.0.1', …)` — the agent's SO_MARK-stamped leg-S dial (`dial_leg_s`, sourced from `host_addr` = `10.99.0.1`, the per-netns gateway) reached S inside `ovd-ns-0000`. ✓
- `RECV 81 bytes` — **the request reached S byte-exact** (81 = `REQUEST.len()`). The C→S decrypt pump works through the real netns/veth topology. ✓
- `SENDALL 81 bytes done` — **S generated and sent its reply.** But the reply IS the 81-byte request bytes (`c.sendall(buf)` — echo). ✗ (this is the defect — the server's reply content)

### 4.2 leg-S wire (`tcpdump -i ovd-hv-0000`): the reply traverses leg-S into the netns and is ACKed by the agent

```
10.99.0.1.42920 > 10.99.0.2.18941: [S]               (agent leg-S dial SYN)
10.99.0.2.18941 > 10.99.0.1.42920: [S.]              (S SYN-ACK)
10.99.0.1.42920 > 10.99.0.2.18941: [.]               (ACK)
10.99.0.1.42920 > 10.99.0.2.18941: [P.] length 81    (agent → S: decrypted REQUEST)
10.99.0.2.18941 > 10.99.0.1.42920: [.] ack 82
10.99.0.2.18941 > 10.99.0.1.42920: [P.] length 81    (S → agent: the 81-byte reply)  ← REPLY PRESENT
10.99.0.1.42920 > 10.99.0.2.18941: [.] ack 82        (agent ACKs the reply)
10.99.0.2.18941 > 10.99.0.1.42920: [F.]              (S closes)
10.99.0.1.42920 > 10.99.0.2.18941: [.] ack 83        (agent ACKs FIN)
9 packets captured
```

The reply leaves S and is **read+ACKed by the agent's leg-S socket.** `run_encrypt_pump`'s `read(leg_s)` gets those 81 bytes. The veth-crossing leg-S (a keystone-vs-baseline diff) carries the reply correctly — ruled OUT as a cause. ✓

### 4.3 leg-C client wire (`ip netns exec ovd-ks-cli-ns tcpdump`): the ENCRYPTED reply reaches the client and is ACKed

The client-netns capture (what the client actually sees on leg-C):

```
10.99.200.2.42540 > 10.99.0.2.18941: [S]                          (client connect → TPROXY → leg-C)
10.99.0.2.18941 > 10.99.200.2.42540: [S.] / [.] ack               (leg-C SYN-ACK)
10.99.200.2.42540 > 10.99.0.2.18941: [P.] len 249                 (TLS client flight 1)
10.99.0.2.18941 > 10.99.200.2.42540: [P.] len 1373                (TLS server cert chain)
10.99.200.2.42540 > 10.99.0.2.18941: [P.] len 1120               (client cert + Finished)
… handshake completes …
10.99.200.2.42540 > 10.99.0.2.18941: [P.] len 103                 (client's ENCRYPTED request, 81→103 TLS record)
10.99.0.2.18941 > 10.99.200.2.42540: [P.] len 103                 (agent's ENCRYPTED reply, 81→103 TLS record)  ← REPLY ON THE WIRE
10.99.200.2.42540 > 10.99.0.2.18941: [.] ack 1477                 (client ACKs the encrypted reply)
13 packets captured
```

`ss -tie` on leg-C confirms the kTLS-TX leg: `tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 … txconf: sw`, `data_segs_out:2` (handshake + reply). **The agent's leg-C kTLS-TX encrypted S's reply and put it on the client wire (the 103-byte 0x17 record); the client's kernel ACKed it.** The asymmetric client-netns return path (a keystone-vs-baseline diff) carries the reply correctly — ruled OUT as a cause. ✓

### 4.4 Client read loop (the consumer): receives the reply byte-exact — but it's the REQUEST, not the RESPONSE

Instrumented client read loop, real output:

```
[DIAG client] REQUEST written observed_rst=false
[DIAG client] read Ok(81); total now 81
[DIAG client] read WouldBlock/TimedOut kind=WouldBlock; got=81
[DIAG client] FINAL got.len()=81 expected=87 byte_exact=false
              got_prefix="OVERDRIVE_CANONICAL_ADDR_REQUEST_client_to_serve"
```

**The client decrypts 81 bytes byte-exact — and they are the REQUEST** (`got_prefix = "…_REQUEST_client_to_serve…"`). The test expects 87 bytes of `RESPONSE`. The production decrypt → deliver → client-read pipe is FLAWLESS (81 bytes in, 81 bytes out, byte-exact). The mismatch is purely *content*: echo of REQUEST vs the distinct RESPONSE the assertion demands.

### 4.5 The arithmetic that settles it

```
REQUEST  = "OVERDRIVE_CANONICAL_ADDR_REQUEST_client_to_server_must_arrive_plaintext_at_S_0302"   (81 bytes)
RESPONSE = "OVERDRIVE_CANONICAL_ADDR_RESPONSE_server_reply_rides_back_over_legC_ktls_to_client_0302" (87 bytes)
client received: 81 bytes == REQUEST   →   got == REQUEST,  got != RESPONSE
```

The echo server (`c.sendall(buf)`) replies with the 81-byte REQUEST; `received_response_byte_exact = (got == RESPONSE)` is `false` because `got` is the REQUEST (81) not the RESPONSE (87).

---

## 5. Five-Whys (multi-causal), each level with PASTED evidence

### Branch A — the dominant cause (the failing assertion)

```
WHY 1A: The client's read loop returns 81 bytes that do NOT equal the 87-byte RESPONSE.
        [Evidence: "[DIAG client] read Ok(81); total now 81" then "FINAL got.len()=81
         expected=87 byte_exact=false got_prefix=\"…_REQUEST_client_to_serve…\"".]
  WHY 2A: The 81 bytes the client received are the REQUEST, not the RESPONSE — the client
          read its OWN request echoed back.
        [Evidence: got_prefix begins "OVERDRIVE_CANONICAL_ADDR_REQUEST…"; REQUEST.len()==81,
         RESPONSE.len()==87; client received exactly 81 bytes.]
    WHY 3A: The server workload S echoes the bytes it received instead of replying with a
            distinct RESPONSE constant.
        [Evidence: server log "RECV 81 bytes … SENDALL 81 bytes done"; the echo script body is
         `buf = c.recv(4096); if buf: c.sendall(buf)` — it returns `buf` (the REQUEST) verbatim;
         server_service_spec, canonical_address_inbound_walking_skeleton.rs:621-654.]
      WHY 4A: The keystone's server spec was authored as a generic TCP echo, but the keystone's
              assertion (`received_response_byte_exact = got == RESPONSE`) demands a reply byte
              string DISTINCT from the request.
        [Evidence: assertion at :841-847 compares `got == RESPONSE`; RESPONSE (:120) is a
         different constant from REQUEST (:116); the echo can never produce RESPONSE.]
        WHY 5A (ROOT CAUSE A): The keystone reused a generic echo-server spec where the
              proven-working reply-leg baseline (cb7d8d09 `inbound_server_run`) used a server
              that READS the request then WRITES a DISTINCT response constant
              (`tcp.write_all(INBOUND_RESPONSE)`). The keystone's two-distinct-constant assertion
              was carried over from that baseline, but the server was not — so the assertion's
              REQUEST≠RESPONSE invariant is unsatisfiable against an echo. A TEST-COMPOSITION
              gap: the test's server logic and its assertion contradict each other.
        [Evidence: cb7d8d09 inbound_server_run: "let request_ok = got == INBOUND_REQUEST;
         tcp.write_all(INBOUND_RESPONSE)…"; keystone echo: "c.sendall(buf)"; both tests carry the
         distinct REQUEST/RESPONSE constants and the `got == RESPONSE` assertion. Only the
         keystone's server fails to produce a distinct RESPONSE.]
```

### Branch B — ruled-OUT production hypotheses (each falsified by §4 wire evidence)

```
WHY 1B (hypothesis): the reply is lost in the production reply pipe (leg-S read /
        leg-C kTLS-TX encrypt / decrypt-on-client) — a NEW production bug like the
        convergence one.
  FALSIFIED at every layer:
  - leg-S read: agent ACKs S's 81-byte reply (§4.2 "[P.] length 81 … [.] ack 82"). ✓ works
  - leg-C kTLS-TX encrypt: 103-byte 0x17 record leaves the agent toward the client
    (§4.3); ss -tie shows leg-C txconf:sw, data_segs_out:2. ✓ works
  - client decrypt: client reads Ok(81) byte-exact (§4.4). ✓ works
  -> The production S→C encrypted reply pipe is PROVEN-WORKING in THIS composition.
     There is no production reply-leg defect.

WHY 1B' (hypothesis): the veth-crossing leg-S (10.99.0.2 in-netns) or the asymmetric
        client-netns return path breaks the reply (keystone-vs-baseline diffs).
  FALSIFIED: leg-S reply traverses ovd-hv-0000 and is ACKed (§4.2); the encrypted reply
  traverses into ovd-ks-cli-ns and is ACKed by the client (§4.3). Both novel topology axes
  carry the reply correctly. ✓ ruled out
```

### Cross-validation

Root Cause A explains EVERY observed symptom with no contradiction: the client receives 81 bytes (not 87) because S echoes the 81-byte REQUEST; `observed_rst == false` because the write and the entire round-trip succeed cleanly (no RST — the connection is healthy, the bytes are just the "wrong" content); the panic is on the SECOND assert (`received_response_byte_exact`), never the first. Branch B's production hypotheses are each independently falsified by the wire captures. No symptom is left unexplained, and no production defect is implicated.

---

## 6. THE DECISIVE CLASSIFICATION — (T) TEST-COMPOSITION GAP

**(T), not (P).** The single settling piece of evidence:

> `[DIAG client] FINAL got.len()=81 … got_prefix="OVERDRIVE_CANONICAL_ADDR_REQUEST_client_to_serve"`
> — the client received the **REQUEST** (81 bytes) byte-exact over the production leg-C kTLS, while the assertion demands the distinct **RESPONSE** (87 bytes).

The production reply pipe delivered bytes byte-exact end to end (server reply → leg-S → leg-C kTLS-TX encrypt → wire → client decrypt). The ONLY reason `received_response_byte_exact` is false is that the keystone's **echo** server returns the request bytes, not a distinct RESPONSE. The proven-working baseline (`cb7d8d09`) drove the SAME production `establish`/`run_encrypt_pump` path and was GREEN — because its server replied with a *distinct* constant. This is a defect in the keystone's server fixture, entirely inside step 03-02's test surface; no production source is implicated.

### Why this is NOT the convergence-bug pattern

The predecessor RCA found a genuine PRODUCTION gap (a `FinalizeFailed{Stable}` teardown of a live netns). That was a defect in production code reached through the production entry points. THIS finding is the opposite: production code is exercised correctly and works byte-exact; the test's own server fixture contradicts the test's own assertion. The two are different classes — do not conflate by analogy.

---

## 7. RECOMMENDED FIX (mapped to Root Cause A) — fits step 03-02

**Make the keystone's server reply with the distinct `RESPONSE` constant after reading the `REQUEST`, exactly as the proven-working `cb7d8d09` baseline did.** Replace the generic echo body in `server_service_spec` (`canonical_address_inbound_walking_skeleton.rs:621-654`) so the server reads the request bytes, then writes `RESPONSE` (not the echoed buffer). Two equivalent shapes:

- **Read-then-reply-RESPONSE (mirrors the baseline `inbound_server_run` — recommended):** the Python one-liner reads up to `REQUEST.len()` bytes, then `c.sendall(RESPONSE_BYTES)` where `RESPONSE_BYTES` is the keystone's `RESPONSE` constant interpolated into the script (the same way `SERVICE_PORT` is interpolated today). This keeps the two-distinct-constant invariant the assertion depends on, and proves the request *and* the reply directions distinctly (the request is checkable separately if desired; the reply is unambiguously the RESPONSE).
- **Assert echo instead (NOT recommended):** change the assertion to compare `got == REQUEST`. Rejected — it weakens the litmus (a reply leg that merely echoes the request proves less than a reply leg carrying a server-authored distinct payload; an echo cannot distinguish "the reply leg works" from "the request was looped back at some layer"). The baseline deliberately used distinct constants to make the reply direction unambiguous; preserve that.

**Scope decision: this fits step 03-02 (the S-WS keystone's own deliverable).** The fix is a change to the keystone's server fixture (its `echo_script`), not to any production source. It restores the keystone to a satisfiable, baseline-shaped litmus. After the fix, the keystone should pass on dev-Lima 7.0 (the full production pipe is already proven working by §4); the merge-blocking signal remains the pinned-6.18 Tier-3 matrix (ADR-0068) per the keystone's own AC.

**Secondary (cosmetic, lower priority — out of this RCA's required scope, surface to the orchestrator):** the post-panic 120 s teardown hang. The panic unwinds before `keystone.shutdown()`, so `Keystone::drop` blocks. Not a correctness defect; if a quick win is wanted, ensure the dial result is asserted *after* an explicit shutdown, or make `Keystone::drop` non-blocking on the panic path. Do NOT bundle this into the Root-Cause-A fix without user direction.

---

## 8. Evidence appendix — provenance & discipline

- All test runs: `cargo xtask lima run -- bash -lc 'timeout 360 cargo nextest run -p overdrive-control-plane --features integration-tests -E "test(workload_reached_at_canonical_address_terminates_mtls_end_to_end)" --no-fail-fast > /tmp/ks-runN.log 2>&1; …'`, each host-`timeout 420`-wrapped, Bash-tool-timeout 450 s, output captured to a guest file and read back host-timeout-wrapped. VM scrubbed (cgroups/netns/veths/nft/bpffs + /tmp logs) before and after every run; left CLEAN (verified: no `ovd-`/`ks-` netns, no `ovd-` links, no `overdrive-mtls` nft table, no `alloc-*.scope`).
- Kernel: `uname -r = 7.0.0-22-generic`.
- Test-side instrumentation (uncommitted, **fully reverted** — the keystone was restored from a pre-instrumentation backup; `git diff` on the file is clean): (a) the echo script logged ACCEPT/RECV/SENDALL/CLOSED to `/tmp/ks-server.log`; (b) `diag_pre_dial`/`diag_post_dial` dumped `ip route get`, nft, `ip rule`, table-100, `ss -tlnp`/`ss -tie`, netns listeners/addrs/conns; (c) `DiagCaptures` ran bounded `tcpdump` on `lo`, `ovd-hv-0000`, `ovd-ks-cli-hv`, and `ip netns exec ovd-ks-cli-ns tcpdump`; (d) the client `dial` read loop printed each `tls.read` outcome + the final `got` prefix. No production source was touched. tcpdumps were reaped by `pkill -f 'tcpdump -i '` (argv-specific, never matching the scrub shell).
- Baseline citations: increment-i findings (`docs/feature/transparent-mtls-host-socket/spike/findings-inbound-intercept.md`, § "What was NOT tested"); the OLD GREEN composed test at commit `cb7d8d09` (`crates/overdrive-worker/tests/integration/bidirectional_walking_skeleton.rs`, `inbound_server_run` + the `received_response_byte_exact` / `records_from_wire_port` assertions).
- Production reply-path source confirmed by read: `crates/overdrive-dataplane/src/mtls/inbound.rs::establish` (step 5 `PumpHandle::spawn_encrypt(leg_s_fd, leg_c_fd, …)` = the S→C reply pump) and `crates/overdrive-dataplane/src/mtls/splice.rs::run_encrypt_pump` (blocking `read(leg_s)` → `write_all(leg_c kTLS-TX)`). This is the SAME path the `cb7d8d09` baseline drove GREEN.
