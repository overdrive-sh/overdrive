# Slice 05 — restart re-handshake + the WASM variant (unconditional); in-flight kTLS survival (spike-gated)

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-05

## Goal (one sentence)

After a node-agent restart, new connections re-handshake and re-install kTLS cleanly
and WASM workloads (in-process host sockets) are encrypted identically to processes
(the UNCONDITIONAL Phase-2 deliverables); AND — spike/DESIGN-gated, NOT an
unconditional acceptance gate — where the workload owns the socket fd and the
sockops link/maps are bpffs-pinned, an in-flight kTLS session survives the restart
(kTLS state is socket-owned).

## IN scope

- (Spike/DESIGN-gated — NOT an unconditional Phase-2 AC) in-flight kTLS session
  survival across a node-agent `kill -9` + restart WHERE the spike confirms the
  composed shape (workload owns the socket fd AND the sockops bpf_link + maps are
  bpffs-pinned) — observable: `ss -K` still shows the kTLS ULP and the workloads
  continue exchanging TLS 1.3 records with record-sequence continuity. If the spike
  does not confirm it, the documented behaviour is new-connection re-handshake.
- A fresh agent re-hydrating its management view from the bpffs pins
  (`PinnedLink::from_pin` / `Map::from_pin`) without re-handshaking the live
  connection.
- A NEW connection opened after the restart re-running the detect→handshake→install
  handoff (held SVID re-read via `IdentityRead`) and carrying TLS 1.3 records.
- The WASM variant: a wasmtime workload (in-process host fd — NO `pidfd_getfd`)
  detected, handshaked, and kTLS-installed identically; `tcpdump` shows TLS 1.3
  records; the WASM workload holds no cert/key.
- (Documented, NOT an AC) a FULL NODE REBOOT wipes all kernel-owned state (bpffs
  pins do not survive a reboot) → every connection re-handshakes from scratch.

## OUT scope

- Multi-node restart / cross-node held sets / node attestation → #36.
- SVID rotation on a long-lived connection (v1 = teardown+reconnect; in-place rekey
  → #229) — the restart path re-handshakes new connections, it does not rekey
  in-place.
- The mechanism CHOICE for fd-ownership (DESIGN's, targeted by the Slice-00 spike).

## Learning hypothesis

- **Disproves if it fails**: "after a restart, new connections re-handshake and
  WASM works identically to process" (the unconditional deliverables) — AND,
  separately and spike-gated, "an in-flight kTLS session survives the restart when
  the workload owns the fd and the sockops link/maps are bpffs-pinned." If new
  connections do not re-handshake or WASM cannot be encrypted, the slice fails. The
  in-flight-survival clause is NOT an unconditional acceptance gate: per the
  CP-restart-survival research §C the composed runtime behaviour (kTLS survives
  configuring-process death with sequence continuity) is empirically open and the
  spike observes it — if it does not hold, the documented behaviour is
  new-connection re-handshake, not a slice failure.
- **Confirms if it succeeds**: encryption is durable across agent restarts (when
  wired for it) and covers both host-socket workload kinds (process + WASM) — the
  feature is complete for host-socket workloads.

## Acceptance criteria

- [ ] (Spike/DESIGN-gated — NOT an unconditional Phase-2 AC) WHERE the spike confirms the composed shape (workload owns the socket fd; sockops bpf_link + maps bpffs-pinned), an in-flight kTLS session survives a node-agent `kill -9` + restart — observable: `ss -K` still shows the kTLS ULP and the workloads continue exchanging TLS 1.3 records with record-sequence continuity (TEST tier, via Lima). If the spike does not confirm it, the documented behaviour is new-connection re-handshake.
- [ ] A fresh agent re-hydrates its management view from the bpffs pins (`PinnedLink::from_pin` / `Map::from_pin`) without re-handshaking the live connection.
- [ ] A NEW connection opened after the restart re-runs the detect→handshake→install handoff (held SVID re-read via `IdentityRead`) and carries TLS 1.3 records.
- [ ] A WASM workload (wasmtime, in-process host fd — no `pidfd_getfd`) is detected, handshaked, and kTLS-installed identically; `tcpdump` shows TLS 1.3 records; the WASM workload holds no cert/key.
- [ ] (Documented, not an AC) a FULL NODE REBOOT wipes all kernel-owned state (bpffs pins do not survive a reboot) → every connection re-handshakes from scratch — stated as expected behaviour, not promised survival.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the restart-survival + WASM acceptance tests (real kernel, `kill -9` the agent — not `--no-run`).

## Dependencies

- Slice 03 (in-flight kTLS sessions to survive) + Slice 01 (the bpffs-pinned sockops
  link/maps).
- The fd-ownership decision (workload owns the socket) — targeted by the Slice-00
  spike; a DESIGN-pinned precondition (CP-restart-survival research §B/§C).
- aya `PinnedLink::from_pin` / `Map::from_pin` (re-hydration) — available in the
  shipped stack.
- wasmtime in-process host sockets (the WASM workload kind) — exists.

## Effort estimate

~1 day (≤6h). Reference class: the restart-survival observation is the
CP-restart-survival research §C minimal experiment (kill -9 → observe `ss -K` + bpffs
pins + live data exchange → fresh agent re-hydrates); the WASM variant differs only
in fd acquisition (in-process host fd vs `pidfd_getfd`).

## Pre-slice SPIKE

Not needed AS A SEPARATE SPIKE — Slice 00's handoff targets the workload-owns-fd
shape precisely so this slice's restart-survival observable is reachable. The
composed runtime behaviour (kTLS survives the configuring-process death with
sequence continuity) is observed HERE on the real kernel (per CP-restart-survival
research §C — it cannot be settled by reading).

## Taste-test note

A thin vertical cut: ships restart survival + the WASM variant. Production-data
observable (real `kill -9` → `ss -K` still shows the ULP + live TLS 1.3 exchange
continues; WASM `tcpdump` shows TLS 1.3 — Tier-3). Disproves a real assumption (kTLS
state survives an agent restart when wired for it; WASM works like process). One
value story (US-MTLS-05); the observable is the surviving session + the WASM wire
capture — the durability + workload-kind completeness of the feature, not
infra-only. The full-node-reboot honesty (documented, not promised) keeps the
durability claim from overstating.
