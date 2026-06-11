# Slice 03 — kTLS install on the workload's socket, agent exits, wire carries TLS 1.3 (the North-Star observable)

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-03

## Goal (one sentence)

After the handshake (Slice 02), the agent installs the rustls handshake's extracted
secrets into kTLS on the workload's own socket (`setsockopt TCP_ULP "tls"` +
`TLS_TX/TLS_RX` — auth-session == data-session) and EXITS the data path, so the
kernel does the steady-state record framing + crypto autonomously — and a `tcpdump`
capture on the veth proves the wire carries TLS 1.3 records.

## IN scope

- Installing the rustls handshake's extracted secrets into kTLS on the workload's
  own socket (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's
  secrets (auth-session == data-session), not a separately negotiated session.
- The agent EXITING the data path after install — the kernel does the steady-state
  record framing + crypto; the agent does no per-byte work.
- `ss -K` showing the kTLS ULP installed on each workload's socket.
- A `tcpdump` capture on the veth between two host-socket workloads showing TLS 1.3
  Application Data records (content type 0x17) and NEVER the cleartext payload —
  **the K1 North-Star observable**.

## OUT scope

- The fail-closed-on-absent/wrong-SVID negative path + the race-window
  no-cleartext-before-install proof → Slice 04.
- Restart survival of in-flight kTLS sessions + the WASM variant → Slice 05.
- The kTLS `crypto_info` struct construction details are DESIGN's to pin (DISCUSS
  pins the observable, not the struct).

## Learning hypothesis

- **Disproves if it fails**: "the negotiated session installs into kTLS on the
  workload's own socket (auth-session == data-session), the agent exits the data
  path, and the wire carries TLS 1.3 records — encryption is in-kernel with the
  agent out of the steady-state path." If the install fails, the agent cannot exit,
  or the wire shows cleartext / a userspace proxy stays in the path, the "in-kernel,
  agent out of the path" property (the thing distinguishing Overdrive from ztunnel)
  does not hold.
- **Confirms if it succeeds**: the North-Star observable (TLS 1.3 on the wire,
  in-kernel, agent out) is achieved; Slices 04/05 add the guards (fail-closed,
  no-cleartext, restart) and the WASM variant on top.

## Acceptance criteria

- [ ] The agent installs the rustls handshake's extracted secrets into kTLS on the workload's own socket (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's secrets (auth-session == data-session), not a separately negotiated session.
- [ ] After install the agent EXITS the data path — the kernel does the steady-state record framing + crypto; the agent does no per-byte work.
- [ ] `ss -K` shows the kTLS ULP installed on each workload's socket (TEST tier, via Lima).
- [ ] A `tcpdump` capture on the veth between two host-socket workloads shows TLS 1.3 Application Data records (content type 0x17) and NEVER the cleartext payload (TEST tier, gated `integration-tests`, via Lima) — the K1 North-Star observable.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the wire-capture acceptance test (real kernel, not `--no-run`).

## Dependencies

- Slice 02 (the completed handshake's extracted secrets).
- In-kernel TLS 1.3 TX+RX on the 6.18 kernel (ADR-0068) — guaranteed.
- The Lima/LVH harness with `tcpdump` + `ss` on the real kernel.

## Effort estimate

~1 day (≤6h). Reference class: the kTLS install (`setsockopt TLS_TX/TLS_RX`) was
exercised by the spike; the new part is the agent-exit + the wire-capture/`ss -K`
acceptance harness.

## Pre-slice SPIKE

Not needed — Slice 00 (the spike) exercised the kTLS install + wire capture on the
real kernel. This slice productionises it.

## Taste-test note

The North-Star slice: ships the kTLS install + agent-exit + the headline wire
capture. Production-data observable (real `tcpdump` TLS 1.3 records + `ss -K` ULP —
Tier-3, the security-reviewer-facing proof). Disproves a real assumption (the
encryption is in-kernel with the agent OUT of the path, not a userspace proxy). One
value story (US-MTLS-03); the observable IS the value (principle 3 made real on the
wire) — the headline deliverable, not infra-only.
