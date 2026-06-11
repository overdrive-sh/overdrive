# Slice 00 — SPIKE (walking skeleton): prove in-band sidecarless kTLS race-free on the pinned kernel for ONE flow

> **SPIKE — the deliverable is EVIDENCE + a VERDICT, not production code.** This
> is the walking skeleton (D2 spike-first): the issue MANDATES a Tier-3 spike
> before the design locks because the core mechanism (in-band sidecarless kTLS,
> auth-session == data-session, agent exits the data path) is **unshipped
> anywhere**. The mechanism is NOT pinned in DISCUSS — this spike settles the
> riskiest input for the DESIGN wave.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-00 `@spike`
**Walking skeleton**: YES — the thinnest end-to-end cut, on ONE process→process flow
**Type**: Spike (time-boxed ~2 days; fixed duration; clear learning objective)

## Goal (one sentence)

On the pinned 6.18 Lima/LVH kernel, prove (or disprove) that ONE process→process
host-socket flow can go through the full in-band handoff — `sockops
ACTIVE_ESTABLISHED → pidfd_getfd → rustls TLS 1.3 handshake presenting the held
SVID (read via IdentityRead) and verifying the peer → kTLS install (setsockopt
TLS_TX/TLS_RX) → agent exits → tcpdump shows TLS 1.3 records` — race-free (no
cleartext byte before kTLS install).

## IN scope

- A Tier-3 spike harness on the 6.18 kernel driving ONE process→process host-socket
  flow through the full handoff (detect → acquire fd → handshake presenting the
  held SVID → kTLS install → agent exits).
- Reading the held SVID + trust bundle via the shipped `IdentityRead` port (#35) —
  NO spike-local issuance or cache.
- A `tcpdump` wire capture on the veth proving TLS 1.3 Application Data records
  (0x17), and `ss -K` proving the kTLS ULP installed.
- A race-window probe: `write("PLAINTEXT_PROBE")` immediately on `connect()` return
  + a deliberately delayed kTLS install (e.g. 500ms) → `tcpdump` never shows the
  plaintext probe string; a sk_msg drop-counter signals whether the window fired.
- An explicit **PASS/FAIL verdict** recorded; on FAIL, the failure mode is named and
  the documented Cilium fallback (out-of-band auth + separate encryption / userspace
  proxy à la Architecture C) is selected.
- The handoff targets the **workload-owns-fd** shape (so Slice 05 restart-survival
  is reachable).

## OUT scope

- Production code (this is a spike — the deliverable is evidence + a verdict).
- The passive (server) side, server-speaks-first protocols, WASM — Slices 01–05.
- The full fail-closed-on-wrong-SVID negative path — Slice 04 (the spike covers the
  no-cleartext-before-install race-window proof only).
- Multi-flow / concurrency, restart survival, the WASM variant — later slices.
- The mechanism CHOICE for production (in-band vs proxy vs fallback) — DESIGN's,
  informed by THIS spike's verdict.

## Learning hypothesis

- **Disproves if it fails**: "in-band sidecarless kTLS is achievable race-free on
  the 6.18 kernel for one process flow — sockops detect → pidfd_getfd → rustls
  handshake presenting the held SVID → kTLS install → agent exits → tcpdump shows
  TLS 1.3, with no cleartext before install." If the handoff cannot be made
  race-free for one flow, in-band sidecarless kTLS is disproven for Overdrive and
  the design MUST adopt the Cilium fallback (out-of-band auth + separate
  encryption). **A FAIL verdict is a SUCCESSFUL spike outcome** (it answered the
  question) — not a failure of the slice.
- **Confirms if it succeeds**: the riskiest mechanism assumption holds; Slices
  01–05 productionise the proven handoff (detect → handshake → install → guards →
  durability) on a validated mechanism.

## Acceptance criteria

- [ ] On the 6.18 Lima/LVH kernel, the spike drives ONE process→process host-socket flow through `sockops ACTIVE_ESTABLISHED → pidfd_getfd → rustls TLS 1.3 handshake presenting the held SVID (read via `IdentityRead`) and verifying the peer → kTLS install (`setsockopt TLS_TX/TLS_RX`) → agent exits`.
- [ ] A `tcpdump` capture on the veth shows TLS 1.3 Application Data records (content type 0x17) and NO cleartext payload; `ss -K` shows the kTLS ULP installed on the socket.
- [ ] A race-window probe (write immediately on connect() return + deliberately delayed kTLS install) shows NO cleartext byte on the wire before install — `tcpdump` never contains the plaintext probe string; a drop-counter signals whether the window fired.
- [ ] An explicit PASS/FAIL verdict is recorded; on FAIL it names the failure mode and selects the documented Cilium fallback (out-of-band auth + separate encryption / userspace proxy) — a successful spike outcome either way.
- [ ] The handoff targets the workload-owns-fd shape (so Slice 05's restart-survival observable is reachable); the verdict + the wire capture are recorded as the spike deliverable.
- [ ] `cargo xtask lima run -- ...` (the spike harness runs on the real 6.18 kernel via Lima/LVH — NOT `--no-run`, which would not execute the runtime handoff).

## Dependencies

- The shipped `IdentityRead` port + `Arc<IdentityMgr>` held store (#35) — exists.
- The shipped `Ca` hierarchy + `SvidMaterial` (cert + leaf key, ADR-0063 D9) (#28) — exists.
- The pinned 6.18 LTS kernel (ADR-0068) on Lima/LVH — guaranteed in-kernel TLS 1.3
  TX+RX + `CONFIG_NET_HANDSHAKE`.
- `pidfd_getfd` + sockops + kTLS on the real kernel (no Tier-2 backstop —
  `BPF_PROG_TEST_RUN` is unavailable for the relevant socket-context hooks; this can
  only be settled at Tier 3).

## Effort estimate

~2 days (time-boxed spike). The cost is the real-kernel handoff plumbing (sockops →
pidfd_getfd → rustls → kTLS) and the wire-capture/race-window observation harness,
NOT production hardening.

## Pre-slice SPIKE

This slice IS the spike. It is the ONE place a spike is the right tool: the
mechanism is genuinely unshipped (no production precedent for in-band sidecarless
kTLS — race-window research Finding 6, CP-restart research Gap 1) and has no Tier-2
backstop (`BPF_PROG_TEST_RUN` unavailable for socket-context hooks), so it can only
be settled at Tier 3 on the real pinned kernel.

## Taste-test note

The walking skeleton, deliberately thinned to ONE process→process flow + a verdict.
Touches every backbone activity (detect → handshake → install → prove) on one flow.
Production-data observable (real `tcpdump` TLS 1.3 records + `ss -K`, NOT synthetic).
Disproves a real pre-commitment (the in-band sidecarless kTLS mechanism the design
would otherwise lock on). Carries one `@spike` story (US-MTLS-00) — a spike, not an
infra-only shell; its observable is the wire capture. A FAIL verdict is a valid,
valuable outcome (it selects the documented fallback) — the spike is honest about
the riskiest assumption rather than assuming it.
