# Root Cause Analysis — S-04-C reverse-NAT UDP e2e "CAPTURE FAILED / tcpdump saw nothing"

**Date**: 2026-06-04
**Analyst**: Rex (nw-troubleshooter)
**Problem**: CI test failure —
`overdrive-dataplane::integration
reverse_nat_udp_e2e::missing_backend_response_distinguished_from_wrong_source`
fails on the intrinsic positive-control assertion (`query_datagrams_captured > 0`).
**Verdict**: **Test-infrastructure defect, NOT a production regression.** The
assertion is correct to refuse a vacuous pass; the *capture mechanism it guards
with* is non-deterministic.

---

## TL;DR

The failing assertion is the **positive control**, not the actual distinguisher:

```
CAPTURE FAILED / tcpdump saw nothing — cannot trust the silence.
```

The any-source `tcpdump` on the client veth witnessed **zero** outbound query
datagrams (`dport 5353`). The control fired exactly as designed — it refused to
treat an empty `reply_source_ips` as a genuine "no reply" when the capture
itself could not be trusted.

The capture could not be trusted because **tcpdump's readiness is synchronized
only by a fixed `sleep(300ms)`**, and the liveness witness is a **single,
un-retransmitted UDP datagram**. Under CI load the AF_PACKET socket is not yet
bound when that one datagram egresses → it is missed → witness = 0 → the control
fails closed.

Production is fine: **S-04-A and S-04-B passed in the same run** (forward +
reverse UDP NAT works on the real kernel; their load-bearing observable is the
kernel-redirected `nc` reply, independent of tcpdump). The witness datagram
egresses *before* the forward XDP rewrite, so it carries **no production signal**
whatsoever — a zero witness can only mean "the capture wasn't live."

---

## Evidence

| Fact | Source |
|---|---|
| Failing assertion is `query_datagrams_captured > 0` (the positive control), at `reverse_nat_udp_e2e.rs:871` | CI log + test source |
| `[diag] ... reply source IPs = [], outbound query witness datagrams (dport 5353) = 0` | CI stderr |
| `[diag] client nc datagram 0: stdout=[] stderr=[]` (nc did not error; no reply, expected for backend-down) | CI stderr |
| S-04-A and S-04-B **passed** in the same run (230 passed, 1 failed) — real-kernel forward+reverse UDP path works | CI summary |
| Positive control is the **most recent** change to the file: `bbd12462 test(udp-service-support): intrinsic capture positive-control in S-04-C` | `git log` |
| Capture readiness = `std::thread::sleep(Duration::from_millis(300))` only; no readiness gate | `reverse_nat_udp_e2e.rs:528` |
| tcpdump spawned with `.spawn().ok()` and `stderr(Stdio::null())` — exec failure is **silently swallowed** | `reverse_nat_udp_e2e.rs:526` |
| Witness is one UDP datagram per round-trip; S-04-C sends `count = 1` → exactly **one** capture chance | `run_round_trips(..., 1)` at `:852` |
| TCP sibling `reverse_nat_e2e.rs` treats tcpdump as **best-effort diagnostic only**; its load-bearing assertion is `client_stdout.contains(PAYLOAD)` (`:470`). S-04-C is the **first** test to make the pcap load-bearing. | `reverse_nat_e2e.rs:299-301,470` |
| The test cites `target/probe/probe_query_witness.sh` 3× as empirical proof the witness "is reliably present" — **that script does not exist** (only `probe_capture.sh` is present) | `ls target/probe/` |
| VIP=`10.0.0.1`, CLIENT_IP=`10.0.0.10` → witness line shape `10.0.0.10.<eph> > 10.0.0.1.5353` matches the parser; **the parser is not the fault** | `netns.rs:242,244` + parse fn |

---

## 5 Whys (multi-causal)

**WHY 1 — Why did the test fail?**
The positive-control assertion `query_datagrams_captured > 0` evaluated false: the
client-veth capture saw zero outbound query datagrams. *(Note: the actual
distinguisher `reply_source_ips.is_empty()` was never reached — the control
short-circuits first, by design.)*

**WHY 2 — Why were zero query datagrams captured?**
The any-source `tcpdump` did not observe the single outbound UDP query
(`client → VIP:5353`) that the client sends unconditionally on every round-trip.

**WHY 3 — Why didn't tcpdump observe a datagram that definitely egressed?**
Two non-exclusive mechanisms, both "capture liveness was never verified":

- **3a (primary — startup race):** tcpdump's AF_PACKET socket was not yet bound
  when the datagram egressed. The only synchronization is a fixed
  `sleep(300ms)` after `spawn()`; under CI load (`ip netns exec … tcpdump`
  startup: open pcap, create raw socket, compile filter) this is insufficient.
  Because the witness is a **single, un-retransmitted UDP datagram**, missing the
  startup window means it is missed *forever* (unlike a TCP SYN, which retries).
- **3b (contributing — swallowed setup):** `Command::…spawn().ok()` discards a
  spawn/exec failure and `stderr(Stdio::null())` hides tcpdump's own error
  output. If tcpdump were missing or exec'd non-zero, the capture would be
  silently empty and indistinguishable from 3a. This is the
  `.claude/rules/debugging.md` § 8 anti-pattern ("`let _` / `.ok()` on fallible
  setup is a debt-bomb").

**WHY 4 — Why is a fixed sleep the only synchronization?**
The capture plumbing was modeled on the TCP sibling (`reverse_nat_e2e.rs`),
where tcpdump is **purely a best-effort diagnostic** — the load-bearing
assertion there is the kernel-delivered `nc` reply (`client_stdout`), so a
flaky/empty pcap never fails the test. S-04-C is the first test to **promote the
pcap to a load-bearing observable**, but reused the unsynchronized fixed-sleep
startup without adding the readiness gate that promotion demands.

**WHY 5 — ROOT — Why was an unsynchronized capture promoted to load-bearing?**
Commit `bbd12462` added the positive control specifically to close the
vacuous-pass gap — it correctly recognized the capture could silently fail. But
it guards capture liveness **with the very same racy capture it distrusts**, and
the witness's reliability was "empirically verified" only by an
**uncommitted / now-absent** probe script (`target/probe/probe_query_witness.sh`,
under gitignored `target/`). The race window was therefore never actually
characterized before the assertion went load-bearing in CI
(a `.claude/rules/debugging.md` § 6 stale/irreproducible-measurement failure).
The control is *honest* (fails closed rather than passing vacuously) but
*not robust* (the liveness proof inherits the unreliability it was meant to
detect).

---

## Why this is NOT a production bug

- The query witness egresses the client veth **before** the forward
  `xdp_service_map_lookup` rewrite (the test's own docstring states this). It
  therefore reflects only "did the client send + did tcpdump see it" — **never**
  the correctness of forward/reverse NAT.
- **S-04-A** (`replies_received == 1`) and **S-04-B** (`replies_received == 3`)
  passed in the same CI run. Those assert the kernel-redirected, VIP-source-
  rewritten reply actually reached `nc` — the real production path — and they do
  not depend on tcpdump. The dataplane works.
- Per `.claude/rules/debugging.md` § 11: an empty collection at the consumer is a
  downstream symptom. Here the "producer" of the witness (client send) and the
  "surface" (a live capture) were never confirmed before the absence was read as
  evidence. The control caught its own unreliability — which is the correct
  outcome, just a red CI.

---

## Solutions (address every root + contributing cause)

Land all three in the test fixture (`crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs`). No production change.

### S1 — Make capture readiness deterministic (fixes WHY 3a / 4 / 5)
Replace the blind `sleep(300ms)` with a real readiness gate. Two options, prefer (a):

- **(a) Wait for tcpdump's "listening on" line.** Pipe tcpdump's **stderr** (not
  `Stdio::null`) and block until it prints
  `listening on <iface>, link-type … ` (its emit-after-socket-bound signal),
  with a bounded timeout. Only then send the first datagram. This eliminates the
  race deterministically.
- **(b) Send a small burst, not one datagram.** Emit the query as e.g. 5
  datagrams at 100 ms spacing (UDP, so cheap), so a single missed startup window
  no longer zeroes the witness. Cheapest change; pairs well with (a) as defense
  in depth.

### S2 — Stop swallowing tcpdump setup failure (fixes WHY 3b; `debugging.md` § 8)
Replace `…spawn().ok()` with `…spawn().expect("spawn client-veth tcpdump")` and
capture stderr so a missing/failed tcpdump fails loudly with a diagnosable
message instead of degrading into a silent empty capture that looks identical to
the startup race.

### S3 — Make the witness-reliability evidence reproducible (fixes WHY 5; `debugging.md` § 6)
Either commit `probe_query_witness.sh` to a tracked location (it currently lives
only under gitignored `target/probe/` and is **absent**), or drop the three
dangling docstring citations to it. A load-bearing reliability claim must rest on
reproducible evidence, not a vanished script.

### Backward-chain validation
With S1 in place, tcpdump is provably live before the first send ⇒ the witness
reliably captures ≥ 1 outbound query ⇒ the positive control passes ⇒ the genuine
distinguisher `reply_source_ips.is_empty()` becomes meaningful ⇒ S-04-C passes
deterministically. S2 converts the residual "tcpdump missing" failure mode from
silent-empty into a loud, correct diagnosis. S3 restores the evidence trail. All
five WHY levels are closed; none requires touching production code.

---

## Cross-references
- `.claude/rules/debugging.md` § 3 (inspection-tool gaps look like negative
  evidence — the control's own premise), § 6 (refresh/commit measurements), § 8
  (`.ok()` on fallible setup), § 11 (empty collection is a downstream symptom).
- `.claude/rules/debugging.md` § "Leftover XDP attachments across runs" — a
  secondary suspect if S1/S2 do not resolve it (a stale XDP program on the client
  veth from a SIGKILL'd prior run could also swallow the egress; run the
  detection one-liner before re-running if the fix does not take).
- Test under analysis: `crates/overdrive-dataplane/tests/integration/reverse_nat_udp_e2e.rs:839-916`.
- Capture mechanism: same file, `run_round_trips` `:479-588`.
- TCP precedent (best-effort tcpdump): `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs:299-301,470`.
