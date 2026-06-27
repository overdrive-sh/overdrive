# Root-Cause Analysis — dial-by-name agent-originated mTLS round-trip "stall"

**Specialist:** Rex (Toyota 5-Whys RCA) · **Date:** 2026-06-27 · **Kernel:** Lima `7.0.0-22-generic` (root)
**Subject test:** `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs::deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls` (S-DBN-WS, one of 4 `#[ignore]`'d ATs)
**Verdict:** **The production datapath is CORRECT and complete end-to-end. The "stall" is a TEST-HARNESS MODEL ERROR — the test client speaks full rustls TLS toward `F`, but the captured leg (leg-F) is the agent's PLAINTEXT workload-facing leg, so the test opens a second, peerless TLS session the agent never terminates.** This is hypothesis **(e) — something else**, and it *falsifies* (a) pump/concurrency deadlock, (b) leg-C not accepting, (c) leg-S stall, and (d) self-dial socket collision.

---

## Problem definition & scope

After commit `82202670` (REV-5 output-hook leg-B interception + the SAN/EKU fixture fixes), the dial-by-name walking skeleton is clean up to the mTLS handshake but the end-to-end round-trip "does not COMPLETE": the agent-originated leg-B→leg-C handshake is reported as stalling (no agent-side rejection, no error), and the test's rustls dial never reads its byte-exact reply. The brief enumerated five candidate causes (a–e) and asked for the precise stall location, the 5-Whys chain, the single smallest production fix, and a confirming probe.

**Scope boundary.** In-scope: the cross-workload dial-by-name path (test client → `F` → egress capture → resolve → leg-B → output-hook → leg-C → leg-S → workload). Explicitly NOT re-derived (proven, trusted as given): the output-hook datapath (pwru-proven, `82202670`), cert identities (SAN/EKU fixed), `resolve(F)→Mesh(10.99.0.2)`, the layer-2 `10.99.0.2` advertise, and the GREEN peer-originated keystone `canonical_address_inbound_walking_skeleton.rs`.

---

## The decisive evidence — population diff (debugging.md §5)

A `[DIAG]` plaintext control dial was added next to the test's existing rustls dial — same netns, same `F`, same port, same egress capture; the ONLY difference is the application-layer protocol. Captured under Lima root:

```
[02-02] getent ahostsv4 server.svc.overdrive.local in ovd-ns-0001 -> [10.98.0.1] (code Some(0))
[02-02] resolved STABLE frontend F = 10.98.0.1
[DIAG] plaintext control: sent REQUEST (81 bytes), got 80 bytes byte_exact_RESPONSE=true ascii="OVERDRIVE_DIAL_BY_NAME_RESPONSE_server_reply_rid"
[DIAG] test-client TCP connected: local=10.99.0.6:56468 peer=10.98.0.1:18951
[02-02] dial handshake failed (agent leg)
[DIAG] post-handshake-failure raw read err: Resource temporarily unavailable (os error 11)
```

Read it directly:

1. **PLAINTEXT dial to `F` round-trips `RESPONSE` byte-exact.** The entire production datapath fires correctly: connect to `F=10.98.0.1:18951` from the client netns → captured by the production egress nft-TPROXY → leg-F → `resolve(F)→Mesh(10.99.0.2)` → agent leg-B mTLS **client** handshake → REV-5 OUTPUT-hook divert → leg-C mTLS **server** handshake → kTLS arm → leg-S dial to the Python plaintext server at `10.99.0.2:18951` → request spliced → server replies → reply rides back over leg-C kTLS → leg-F → test client. The agent's leg-B→leg-C mTLS session **established** and carried the byte-exact reply.
2. **Full-rustls TLS dial to the SAME `F` fails its handshake** — `dial handshake failed (agent leg)` — and a subsequent raw read returns `EAGAIN` (no ServerHello, no bytes waiting).
3. The two dials are *identical* but for the protocol. Plaintext succeeds; TLS stalls. **The datapath is not the variable; the client's protocol is.**

This single juxtaposition kills four of the five hypotheses (a pump deadlock / a non-accepting leg-C / a stalled leg-S / a self-dial collision would *also* break the plaintext dial — they don't, because plaintext completes).

---

## Why the TLS dial cannot complete — the topology

The dial-by-name egress path inserts the agent's OWN mTLS session *between* the dialing workload and the destination. By ADR-0072 / the whitepaper workload-identity model, **workloads are identity-unaware and speak ordinary plaintext sockets**; the agent's eBPF/TPROXY legs originate all TLS. Concretely:

```
test client ──plaintext?──▶ leg-F (PLAINTEXT, agent-owned)
                              │  agent reads plaintext, re-encrypts
                              ▼
                            leg-B (agent mTLS CLIENT) ──TLS──▶ leg-C (agent mTLS SERVER)
                                                                 │ decrypt, splice
                                                                 ▼
                                                               leg-S ──plaintext──▶ Python server
```

- **leg-F is plaintext** (`mtls_intercept_worker.rs` — the OUTBOUND/workload-facing leg; the forward pump is `run_encrypt_pump`: `read(leg_f) → write_all(leg_b kTLS-TX)`, `splice.rs:390`). The agent never runs a TLS *server* handshake on leg-F.
- `outbound::establish` (`overdrive-dataplane/src/mtls/outbound.rs:59`) first `drain_prearm(leg_f, 250ms)` (`:73`) — it reads whatever the workload wrote first and treats it as **plaintext to encrypt**. When the test client speaks TLS, its ClientHello bytes become leg-B's first kTLS-TX application_data record (`:128`, the `prelude`).
- So a full-rustls test client gets **no TLS peer**: leg-F doesn't answer a handshake, and its ClientHello is tunnelled (as opaque plaintext) inside the agent's leg-B→leg-C session to the Python plaintext server. The test client's rustls `drive_client_handshake` (`dns_responder_walking_skeleton.rs:766`) loops on `read_tls` waiting for a ServerHello that never arrives → handshake fails / RST.

The keystone works because its topology is single-session: a peer-originated connect to the server's `workload_addr` is captured at **prerouting** and lands directly on **leg-C** (the agent's mTLS *server*), so the external rustls peer legitimately plays the originating-agent's leg-B and its TLS terminates at leg-C. (`canonical_address_inbound_walking_skeleton.rs` — the working reference.) **dial-by-name's egress path is structurally different: the capture is at leg-F (plaintext), not leg-C.**

---

## Toyota 5-Whys

```
PROBLEM: The S-DBN-WS test's rustls dial to F never completes its handshake /
         reads its byte-exact reply, while the production datapath is clean
         (no agent-side mTLS rejection).

WHY 1 (Symptom): The test client's rustls ClientConnection handshake to (F,18951)
  does not complete; drive_client_handshake returns false → observed_rst → panic.
  [Evidence: probe `[02-02] dial handshake failed (agent leg)`; raw read EAGAIN
   (no ServerHello) — dns_responder_walking_skeleton.rs:766,843.]

  WHY 2 (Context): There is no TLS server answering the test client's handshake.
    [Evidence: the connection is captured at leg-F (peer=10.98.0.1:18951 confirmed),
     and leg-F is the agent's PLAINTEXT workload-facing leg — the agent runs no
     server handshake there; outbound::establish drains leg-F as plaintext
     (outbound.rs:73,128).]

    WHY 3 (System): The dial-by-name egress path inserts the agent's OWN mTLS
      session (leg-B→leg-C) between the dialing workload and the destination; the
      agent terminates the ONLY TLS, and the workload leg is plaintext by design.
      [Evidence: the PLAINTEXT control dial to the SAME F round-trips RESPONSE
       byte-exact — proving the agent's leg-B→leg-C mTLS established and the whole
       capture→resolve→leg-B→output-hook→leg-C→leg-S→reply loop works
       (probe: `plaintext control … byte_exact_RESPONSE=true`).]

      WHY 4 (Design): Overdrive's workload-identity model makes workloads
        identity-unaware plaintext speakers; the kernel/agent legs do all TLS
        (ADR-0072; whitepaper "Workload identity model — workloads hold NOTHING").
        A dialing workload connects in plaintext; the agent originates mTLS on
        leg-B. The egress capture is therefore at a plaintext leg, NOT a TLS leg.
        [Evidence: leg-F forward pump is read→write_all of PLAINTEXT into leg-B's
         kTLS-TX (splice.rs:386-468); the workload never holds SVID material.]

        WHY 5 (Root Cause): The S-DBN-WS test was written with the KEYSTONE's
          client model — a full rustls ClientConnection whose TLS terminates at the
          captured peer — but the dial-by-name EGRESS capture lands on leg-F
          (plaintext), not leg-C (TLS). A plaintext-terminating capture leg cannot
          terminate a rustls handshake. The keystone's dial model was reused
          verbatim for a path whose capture leg has the OPPOSITE encryption role.
          [Evidence: dns_responder_walking_skeleton.rs:809-880 TestPkiHandle::dial
           runs a full rustls handshake + writes REQUEST as TLS app-data; the
           keystone canonical_address_inbound_walking_skeleton.rs:1016-1090 is the
           SAME dial shape — correct there (capture = leg-C) but wrong here
           (capture = leg-F).]

ROOT CAUSE (e): TEST-MODEL MISMATCH. The test client speaks full rustls TLS toward
  a frontend whose egress capture terminates on the agent's PLAINTEXT leg-F, so the
  test opens a second, peerless TLS session the agent is not designed to terminate.
  The production datapath (egress capture · resolve · leg-B mTLS · REV-5 output-hook
  divert · leg-C mTLS · leg-S splice · reply) is correct and complete.

SOLUTION: Change the S-DBN-WS dial client to speak PLAINTEXT (send REQUEST, read
  RESPONSE) over an ordinary TcpStream to (F, SERVICE_PORT) — modelling a real
  identity-unaware workload — and assert the byte-exact RESPONSE rides back. This
  is the model the production path actually serves; it is also what the keystone's
  leg-S/Python server already expects (a plaintext request, a plaintext reply).
```

### Cross-validation (backwards chain)

- **Forward trace of the root cause → symptom:** if the test speaks full TLS toward a plaintext leg-F, then (i) leg-F answers no handshake, (ii) the ClientHello is tunnelled as plaintext to a plaintext Python server, (iii) the rustls client waits for a ServerHello that never comes → handshake fail/RST. Matches the observed symptom exactly (incl. the `EAGAIN` no-ServerHello read and the prior "Transport RST" observation).
- **No contradiction with the proven facts:** the output-hook datapath (pwru), cert identities, `resolve(F)`, and the `10.99.0.2` advertise are all *exercised and confirmed working* by the plaintext control dial — the RCA builds on them rather than re-deriving them.
- **All symptoms explained:** the "stall" (no ServerHello → 8s read-timeout loop), the "no agent-side rejection" (the agent's own leg-B/leg-C succeed; nothing to reject), and the RST (rustls aborts on the non-handshake bytes) are one mechanism.

---

## The single smallest production fix

**Nothing in production code changes.** The datapath is correct; the fix is to the test harness only.

**Exact site:** `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs`.

**Change:** Replace the full-rustls dial used by `dial_frontend_in_netns` (`TestPkiHandle::dial`, lines ~792–881, and the `TestPkiHandle`/`drive_client_handshake`/PKI-on-the-client-side scaffolding it requires) with a **plaintext** TCP round-trip:

- In the netns, `TcpStream::connect((F, SERVICE_PORT))`, `set_nodelay`, `write_all(REQUEST)`, then read until `RESPONSE.len()` with the existing 8s budget; set `received_response_byte_exact = (got == RESPONSE)` and `observed_rst` on a write/RST error.
- Drop the client-side rustls config, SNI, `ClientConnection`, and `drive_client_handshake` from the dial path. The server-side `mtls_identity_override`/`HeldServerIdentity` PKI **stays** (the agent's leg-B/leg-C still need the SVIDs + bundle); only the *test client's* TLS goes away.
- Keep the byte-distinct `REQUEST`/`RESPONSE` litmus (already present) so the assertion still proves the real S→C reply pipe, not an echo.

This is the model the production path serves and the model the keystone's leg-S Python server already speaks. The confirming probe proved this exact shape (plaintext REQUEST → byte-exact RESPONSE) passes end-to-end on the real kernel.

**Apply to all four `#[ignore]`'d S-DBN ATs** that dial `F` with the same `TestPkiHandle::dial` (S-DBN-WS, S-DBN-WS-STABLE, S-DBN-SINGLE-SRC, S-DBN-CHURN) — they share the dial helper, so they share the model error and the fix.

**No new public API surface is invented** (CLAUDE.md "Implement to the design"): the change deletes test-side TLS and uses `std::net::TcpStream` only. This is **not** spike-class — it is a test-harness correction proven by the in-place probe.

### A design note to surface (not a blocker)

The four ATs are documented (`dns_responder_walking_skeleton.rs:62-73,121-136`) as if the test client legitimately presents an SVID and the agent's leg-C terminates *its* TLS — the keystone framing. That framing is only valid for the peer-originated INBOUND path (prerouting→leg-C). For the dial-by-name EGRESS path the capture is leg-F (plaintext), so the test-client-presents-TLS premise is structurally wrong. The orchestrator/architect should confirm the corrected test model (plaintext client) matches the intended ADR-0072 east-west semantics before the implementer lands the fix — it does, per the workload-identity model, but the AT docstrings (and `MESH_PEER_SNI`/client-SVID scaffolding) were written to the wrong premise and should be corrected in the same change.

---

## Hypotheses (a)–(e): verdicts

| # | Hypothesis | Verdict | Falsifier |
|---|---|---|---|
| (a) | Pump/concurrency deadlock (`block_on` on blocking pool starves leg-C) | **REFUTED** | Plaintext dial completes the full leg-B→leg-C→leg-S→reply loop concurrently; no deadlock. The `handle_outbound` `block_on` (`mtls_intercept_worker.rs:871`) runs on a blocking-pool thread and the two `establish` calls run on separate blocking threads — proven non-blocking by the plaintext success. |
| (b) | leg-C not accepting the agent-originated connection | **REFUTED** | Plaintext dial's reply rode back over leg-C kTLS — leg-C accepted the diverted leg-B and completed the server handshake. |
| (c) | leg-S onward dial fails/stalls | **REFUTED** | Plaintext dial delivered REQUEST to the Python server at `10.99.0.2:18951` and got RESPONSE back — leg-S reached the workload. |
| (d) | Self-dial socket-identity / loopback collision (single node, agent dials itself) | **REFUTED** | Plaintext self-dial round-trips cleanly; the 4-tuple/fwmark interaction is sound. |
| (e) | Something else | **CONFIRMED** | Test-harness model error: full-rustls client toward a plaintext leg-F. |

---

## Confirming probe — provenance (debugging.md §4, predict-before-probe)

- **Hypothesis:** the test client speaks full rustls toward a plaintext leg-F, opening a second peerless TLS session the agent never terminates; the production datapath is otherwise correct.
- **Predicted:** a PLAINTEXT dial to the same `F` round-trips `RESPONSE` byte-exact, while the full-rustls dial fails its handshake with no ServerHello.
- **Falsification:** if the plaintext dial ALSO failed (no RESPONSE), the defect would be in the leg-B/leg-C/leg-S production path (a, b, or c), not the test model.
- **Result:** prediction confirmed verbatim (`byte_exact_RESPONSE=true`; rustls `dial handshake failed`; raw read `EAGAIN`). Kernel `7.0.0-22-generic`, Lima root.
- **Instrumentation was temporary** (`[DIAG]`-marked): the one AT was un-ignored only for the probe, a plaintext control dial + raw-byte capture + connected-peer log + Python-server recv log were added, the run captured, then **all edits reverted** — `git diff` on the test file is empty and `#[ignore]` is restored. The Lima VM was swept before and after.

---

## Cross-references

- `crates/overdrive-dataplane/src/mtls/outbound.rs:59-145` — `establish`: drains leg-F plaintext, encrypts into leg-B kTLS-TX (the agent originates the TLS).
- `crates/overdrive-dataplane/src/mtls/splice.rs:386-468` — `run_encrypt_pump`: leg-F is read as plaintext.
- `crates/overdrive-worker/src/mtls_intercept_worker.rs:859-919` — `handle_outbound`: resolve→`Enforce`→leg-B mTLS to the resolved backend.
- `crates/overdrive-worker/src/mtls_intercept.rs:164-251,293-373` — `make_transparent_listener` (`IP_FREEBIND`) + REV-5 `install_inbound_tproxy` output divert.
- `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs` — the GREEN peer-originated keystone (single-session, capture = leg-C): the correct reference for "client TLS terminates at the captured leg."
- `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs:792-881` — `TestPkiHandle::dial` (the wrong-model full-rustls client to fix).
- `docs/feature/dial-by-name-responder/feature-delta.md` REV-5 (§2004-2074) — the output-hook datapath contract (correct; not the defect).
- `docs/analysis/root-cause-analysis-dial-by-name-by-frontend-resolve-rst.md` — the prior (datapath) RCA whose fix (`82202670`) the plaintext control dial now confirms WORKS.
