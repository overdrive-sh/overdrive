# E04 — a mesh workload is reachable at its canonical address over mTLS, end to end

**Surface:** E (end-to-end) · **KPI:** K1 · **Status:** `pending`

<!-- Status rationale: `pending` by design. The black-box mesh-mTLS E-surface
capture needs a converged full-system two-workload deployment AND the production
CA → SVID → leg-C mTLS path proven black-box (no in-process test seam, no
injected `mtls_identity_override` / `TestPki`). That requires the EDD harness
(#227 — a disposable full-system Lima VM on the immutable OS) running on the OS
image (#75 — Image Factory MVP). Until both land, `od serve` + two `od deploy`
cannot be driven black-box with a real workload-identity CA, so this expectation
cannot be captured against the built binary. Do NOT cite the stale "serve XDP-on-lo
boot fails" reason — `serve` boots fine post-ADR-0061; the gap is the full-system
two-workload + production-CA black-box harness, not a serve boot failure. The
in-process Tier-3 keystone (`canonical_address_inbound_walking_skeleton.rs`)
covers the round-trip through the production-installed inbound rule with a test
PKI seam; E04 is the black-box operator-observable complement those tiers
under-serve. -->

## Expectation

A mesh workload deployed by the platform is reachable **at its canonical
`workload_addr:service_port`** — the per-workload address the platform
advertises, dialed **directly with no DNS / name lookup** — over **mutual
TLS terminated by the platform**, with the application round-trip completing
byte-for-byte. The dial is captured by the **production-installed** inbound
nft-TPROXY rule (`start_alloc` off `spec.{workload_addr, service_ports}`,
step 03-01), mTLS terminates on the platform's leg-C, and the client's request
reaches the workload and its reply returns — all without the workload holding
any SVID material (kernel-mediated mTLS; the workload is identity-unaware).

This is the genuine operator/peer-observable outcome for the canonical-address
inbound slice: a peer that knows only "the server's canonical address" reaches
the server securely, and the platform — not the workload — does the mTLS.

- Anchor: S-WS (`workload_reached_at_canonical_address_terminates_mtls_end_to_end`, the `@walking_skeleton` keystone in `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`)
- Anchor: roadmap `canonical-workload-address-inbound-tproxy` step 03-02 (S-WS keystone AC — bidirectional mesh loop passes the pinned-6.18 Tier-3 matrix)
- Anchor: GH #241 (Path-A canonical workload address + production inbound mTLS TPROXY install)
- Anchor: docs/feature/canonical-workload-address-inbound-tproxy/design (the canonical-address design + the ADR governing the per-workload `workload_addr` advertise + inbound TPROXY install)

## Verification

Precondition (the deferral): a converged full-system deployment of two mesh
workloads on a real node, with the **production** workload-identity CA issuing
the leg-C/leg-B SVIDs (no `mtls_identity_override` test seam), driven through
the **built `overdrive` binary** (`overdrive serve` + two `overdrive deploy`).
That harness is **#227** (the disposable full-system Lima VM on the immutable
OS) on **#75** (the OS image). Neither has landed, so this expectation is
`pending` and its `runner.sh` self-reports `pending` rather than narrating a
capture it cannot execute.

Sub-claims (to be captured once #227 + #75 land):

1. `overdrive deploy <server-spec>` and `overdrive deploy <client-spec>` both
   exit 0 (`Accepted.`), and the server reaches Running with a materialised
   canonical `workload_addr` (observable via `overdrive alloc status`).
2. A client dialing the server's canonical `workload_addr:service_port`
   **directly** completes a TLS-1.3 handshake whose server cert chains to the
   platform's workload-identity trust bundle, and the application request →
   reply round-trip completes byte-for-byte.
3. (Confidentiality) a wire capture on the leg-C/leg-B path shows TLS-1.3
   `application_data` records (content-type `0x17`) in both directions — the
   bytes are encrypted end-to-end, the plaintext request/response markers never
   appear on the encrypted wire.

`satisfied` requires sub-claims 1–3 captured on a #227 full-system Lima run
against the built binary, reviewed by a different-fox adversarial auditor
reading only the captured `evidence/` (per `.claude/rules/verification.md`).

## Evidence

None captured — `pending`. The `runner.sh` skeleton self-reports `pending`
(the `od serve` + `od deploy ×2` shape is sketched for when #227 + #75 land;
it does NOT narrate or fabricate a capture). Do NOT run
`harness/run-expectation.sh E04` to set `satisfied` before the precondition
exists.

The `what, forever` regression witness for the round-trip through the
production-installed inbound rule is the in-process Tier-3 keystone
`canonical_address_inbound_walking_skeleton.rs` (this expectation is the
black-box operator-observable `why`; the keystone is the in-process `what`).
