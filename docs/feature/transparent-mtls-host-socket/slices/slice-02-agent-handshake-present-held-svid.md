# Slice 02 — the agent performs the TLS 1.3 handshake presenting the held SVID

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-02

## Goal (one sentence)

The node agent performs a rustls TLS 1.3 handshake on the detected workload's
socket, presenting the workload's own held `SvidMaterial` (read via
`IdentityRead::svid_for(&AllocationId)` — #26 is a READER of #35's held set, never
an issuer) and verifying the peer against `IdentityRead::current_bundle()` — with
the workload holding no cert and no key (identity-unaware).

## IN scope

- The node agent performing a rustls TLS 1.3 handshake on the acquired workload
  socket (Slice 01).
- Presenting the held `SvidMaterial` (cert + leaf key, ADR-0063 D9) read via
  `IdentityRead::svid_for(&AllocationId)` (#35) — NO #26-local issuance or cache.
- Verifying the peer against `IdentityRead::current_bundle()` (the hydrated bundle
  behind #35's port); a peer not chaining to it aborts the handshake.
- The presented leaf chaining to the root with SAN = the workload's SPIFFE URI
  (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via `openssl
  verify` / the captured handshake at the TEST tier.
- The workload process holding no cert and no key (the leaf key stays with the
  agent, read via the port).
- #26 taking the `IdentityRead` port as a required constructor parameter
  (port-trait discipline).

## OUT scope

- The kTLS install + agent-exit + wire capture → Slice 03 (this slice ends at a
  completed handshake; the bytes are not yet kTLS-encrypted).
- The full fail-closed-on-absent/wrong-SVID negative proof → Slice 04 (the
  peer-verification abort is here; the absent-SVID-refuses-handshake proof is S04).
- The race window (no cleartext before install) → Slice 04.
- The WASM variant → Slice 05.

## Learning hypothesis

- **Disproves if it fails**: "the agent can perform mTLS on the workload's behalf,
  presenting the workload's own held SVID (read from the single source of truth,
  #35's `IdentityRead`) and verifying the peer — with the workload holding nothing."
  If the agent cannot present the held SVID (e.g. the port read shape is wrong) or
  the workload must hold material, the kernel-mediated / workload-holds-nothing
  model is wrong.
- **Confirms if it succeeds**: the integration seam with #35 (read the held SVID +
  bundle via `IdentityRead`) is sound; the handshake's extracted secrets are ready
  for the kTLS install (Slice 03).

## Acceptance criteria

- [ ] The node agent performs a rustls TLS 1.3 handshake on the detected workload's socket, presenting the held `SvidMaterial` read via `IdentityRead::svid_for(&AllocationId)` (#35) — #26 reads, never mints/caches.
- [ ] The agent verifies the peer against `IdentityRead::current_bundle()` (the hydrated bundle behind #35's port); a peer not chaining to it aborts the handshake (the absent-SVID refusal proof is Slice 04).
- [ ] The presented leaf chains to the root and its SAN is the workload's SPIFFE URI (`spiffe://overdrive.local/job/<name>/alloc/<id>`) — provable via `openssl verify` / the captured handshake at the TEST tier.
- [ ] The workload process holds no cert and no key (the leaf key stays with the agent, read via the port) — the workload is identity-unaware.
- [ ] #26 takes the `IdentityRead` port as a required constructor parameter (never defaulted), per port-trait discipline.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the handshake acceptance test (real kernel).

## Dependencies

- Slice 01 (the acquired workload socket fd).
- The shipped `IdentityRead` port + `Arc<IdentityMgr>` held store + hydrated bundle (#35) — exists.
- The shipped `SvidMaterial` (cert + leaf key, ADR-0063 D9) (#28) — exists.
- rustls TLS 1.3 on the 6.18 kernel (ADR-0068).

## Effort estimate

~1 day (≤6h). Reference class: the rustls handshake is well-trodden; the new part
is the `IdentityRead`-backed cert resolver (read the held SVID by `AllocationId`)
and the peer-verification against the hydrated bundle — both reads of a shipped
port.

## Pre-slice SPIKE

Not needed — Slice 00 (the spike) exercised the handshake-presenting-the-held-SVID
step on the real kernel. This slice productionises it (active side; the passive side
+ protocol nuances are folded into the same path).

## Taste-test note

A thin vertical cut: ships the agent's rustls handshake reading the held SVID +
bundle via `IdentityRead`. Production-data observable (a real completed mutual-TLS
handshake whose presented leaf chains to the root, SAN == the alloc — `openssl
verify` / captured handshake, Tier-3). Disproves a real assumption (the agent
presents the WORKLOAD's identity from the single source of truth, workload holds
nothing). Carries one value story (US-MTLS-02); the observable is the completed,
identity-correct handshake — the integration seam with #35 the whole feature
depends on. Not infra-only.
