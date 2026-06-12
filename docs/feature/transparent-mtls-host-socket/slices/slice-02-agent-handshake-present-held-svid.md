# Slice 02 — the agent handshakes on its peer-facing leg presenting the held SVID (client AND server roles)

> **Re-grounded to ADR-0069 (the agent-light L4 proxy).** The handshake runs on
> the **agent's own peer-facing leg** (leg B outbound, leg C inbound), NOT the
> workload's socket. The `IdentityRead` read is unchanged.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-02

## Goal (one sentence)

On the agent-acquired leg (Slice 01), the agent performs a rustls TLS 1.3
handshake presenting the workload's own held `SvidMaterial` (read via
`IdentityRead::svid_for(&AllocationId)` — #26 is a READER of #35's held set,
never an issuer) and verifying the peer chains to `IdentityRead::current_bundle()`
— as the **CLIENT** on the outbound leg B, and as the **SERVER** on the inbound
leg C (presenting the server SVID AND requiring-and-verifying the client SVID) —
with the workload holding no cert and no key (identity-unaware).

## IN scope

- **Outbound (client role, leg B)**: the agent dials the real peer and performs a
  rustls TLS 1.3 CLIENT handshake presenting the held `SvidMaterial` (cert + leaf
  key, ADR-0063 D9) read via `IdentityRead::svid_for(&AllocationId)`, verifying
  the peer's server cert chains to `IdentityRead::current_bundle()`.
- **Inbound (server role, leg C)**: the agent performs a rustls TLS 1.3 SERVER
  handshake presenting the **server** workload's SVID (selected by the recovered
  original destination → `AllocationId`) AND requiring-and-verifying the client's
  presented SVID chains to the bundle via `WebPkiClientVerifier` (REQUIRE+VERIFY).
- The presented leaf chaining to the root with SAN = the workload's SPIFFE URI
  (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via the captured
  handshake at the TEST tier.
- BOTH workloads holding no cert and no key (the leaf key stays with the agent,
  read via the port; the workload holds only a plaintext socket to the agent).
- #26 taking the `IdentityRead` port as a **required** constructor parameter
  (port-trait discipline — never defaulted).
- Two server-config mechanics that bind on DELIVER (from
  `findings-inbound-intercept.md` § "Mechanics"): suppress `NewSessionTicket`
  (`send_tls13_tickets = 0` — a post-handshake ticket hits `-EIO` on raw
  kTLS-RX); read `peer_certificates()` for the fail-closed guard BEFORE
  `dangerous_extract_secrets()` consumes the connection.

## OUT scope

- The kTLS arm + forward agent-idle splice + return agent-light splice + wire
  capture → Slice 03 (outbound) / Slice 04 (inbound) (this slice ends at a
  completed handshake with extracted secrets ready; the bytes are not yet kTLS).
- The fail-closed-on-absent/wrong-SVID negative proofs (absent SVID outbound;
  nocert/wrongca inbound) → Slice 05 (the peer-verification abort path is
  exercised here; the dedicated negative ACs are S05).
- The honest-claim boundary (chain-to-bundle authn only, NOT intended-peer) +
  resource limits → Slice 05.

## Learning hypothesis

- **Disproves if it fails**: "the agent can perform mTLS on the workload's behalf
  in BOTH roles — presenting the workload's own held SVID (read from #35's single
  source of truth via `IdentityRead`) and verifying the peer chains to the bundle
  — with the workload holding nothing." If the agent cannot present the held SVID
  (e.g. the port read shape is wrong), or the server-side client-auth verifier
  cannot be wired, the kernel-mediated / workload-holds-nothing model is wrong.
- **Confirms if it succeeds**: the integration seam with #35 (read the held SVID +
  bundle via `IdentityRead`) is sound in both directions; the handshake's
  extracted secrets are ready for the kTLS arm (Slices 03 / 04).

## Acceptance criteria

- [ ] **Outbound (client)**: the agent performs a rustls TLS 1.3 CLIENT handshake on leg B presenting the held `SvidMaterial` read via `IdentityRead::svid_for(&AllocationId)` (#35) and verifying the peer chains to `IdentityRead::current_bundle()`. Anchor: `findings.md` A (rustls 1.3 handshake + `dangerous_extract_secrets` drove every spike; real `IdentityRead`/SPIFFE-SAN SVID is the productionisation).
- [ ] **Inbound (server)**: the agent performs a rustls TLS 1.3 SERVER handshake on leg C presenting the server SVID AND requiring-and-verifying the client's SVID chains to the bundle via `WebPkiClientVerifier` (REQUIRE+VERIFY); a valid client cert → handshake succeeds. Anchor: `findings-inbound-intercept.md` §2.
- [ ] The presented leaf chains to the root and its SAN is the workload's SPIFFE URI (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via the captured handshake at the TEST tier.
- [ ] BOTH workloads hold no cert and no key (the leaf key stays with the agent, read via the port) — the workloads are identity-unaware.
- [ ] #26 takes the `IdentityRead` port as a required constructor parameter (never defaulted), per port-trait discipline. The server config suppresses `NewSessionTicket` and reads `peer_certificates()` before `dangerous_extract_secrets()`. Anchor: `findings-inbound-intercept.md` § "Mechanics" #3/#6.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the handshake acceptance test, both roles (real kernel).

## Dependencies

- Slice 01 (the agent-acquired leg F / leg C + the recovered inbound orig-dst).
- The shipped `IdentityRead` port + `Arc<IdentityMgr>` held store + hydrated bundle (#35) — exists.
- The shipped `SvidMaterial` (cert + leaf key, ADR-0063 D9) (#28) — exists.
- rustls TLS 1.3 + `WebPkiClientVerifier` + `dangerous_extract_secrets` on the 6.18 kernel.

## Effort estimate

~1 day (≤6h). Reference class: the rustls handshake (client + server) is
well-trodden and proven in the spikes (`findings.md` A; `findings-inbound-intercept.md`
§2); the new part is the `IdentityRead`-backed cert resolver (read the held SVID
by `AllocationId`) and the server-side client-auth verifier against the hydrated
bundle — both reads of a shipped port.

## Pre-slice SPIKE

Not needed — the spikes exercised the handshake-presenting-the-held-SVID step in
both roles on the real kernel (client in `findings.md`; server-mTLS with
client-auth in `findings-inbound-intercept.md` §2). This slice productionises it.

## Taste-test note

A thin vertical cut: ships the agent's rustls handshake (client + server) reading
the held SVID + bundle via `IdentityRead`. Production-data observable (a real
completed mutual-TLS handshake whose presented leaf chains to the root, SAN == the
alloc; the inbound side verifies the client SVID — captured handshake, Tier-3).
Disproves a real assumption (the agent presents the WORKLOAD's identity from the
single source of truth, in both roles, workload holds nothing). Carries one value
story (US-MTLS-02); the observable is the completed, identity-correct handshake —
the integration seam with #35 the whole feature depends on. Not infra-only.
