# Expectations — master status table

Surfaces: **O** operator CLI · **R** reconciler/convergence · **D** dataplane/kernel · **E** end-to-end · **X** build/supply-chain.
Status: `pending | satisfied | partial | broken | unanchored-claim | out-of-scope` (see `../README.md`).

| ID | Surface | Expectation | KPI | Anchors | Status |
|---|---|---|---|---|---|
| [O01](O01-kind-rejection-guidance/) | O | Job/Schedule + probe rejected with actionable guidance | K5 | S-SHCP-PARSE-05/06, CLI-12..14 | `pending` |
| [O02](O02-alloc-status-probes-section/) | O | `alloc status` renders a Probes section for a Service | K4 | S-SHCP-CLI-01..06 | `pending` |
| [E01](E01-coinflip-service-honest-early-exit/) | E | coinflip-as-Service honest EarlyExit, never `(took live)` | K1 | S-SHCP-RECON-04, INT-CLI-01, CLI-07..11 | `pending` |
| [O03](O03-deploy-udp-service-accepted-udp-intent/) | O | `overdrive deploy <udp-spec>` accepted; intent carries `Proto::Udp` | K1 | S-04-A, roadmap 01-05, ADR-0060, ADR-0061, US-04 | `satisfied` |
| [E02](E02-udp-service-reverse-path-vip-sourced/) | E | deployed UDP service's reply sourced from VIP, not backend IP | K1 | S-04-A, K1, roadmap 01-03, ADR-0060, ADR-0061, US-04 | `pending` (remote-path) |
| [E03](E03-ca-full-chain-verifies/) | E | full Root → Intermediate → SVID chain verifies under `openssl verify` | K1 | S-04-07, ADR-0063 D1, built-in-ca K1 | `pending` |
| [E04](E04-workload-reachable-at-canonical-address-mtls/) | E | a mesh workload is reachable at its canonical `workload_addr:service_port` over mTLS, end to end | K1 | S-WS, roadmap 03-02, GH #241, canonical-address design + ADR | `pending` |
| [O04](O04-ca-refuse-to-start-actionable-error/) | O | control plane refuses to start on root-key decrypt failure with an actionable, cause-distinct error (no silent re-mint) | K3 | S-02-06/07, ADR-0063 D3/Earned-Trust, journey error_paths step 1 | `pending` |
| [O05](O05-ca-issued-certificates-audit-row/) | O | every issuance observable as an `issued_certificates` audit row via `alloc status`; no silent issuance | K1 | S-05-03/04, ADR-0063 D6, journey step 4 | `pending` |
| [D01](D01-ca-root-key-never-plaintext-at-rest/) | D | root CA private key never plaintext at rest (byte-scan IntentStore) | K3 | S-02-02, ADR-0063 D2/D4, built-in-ca K3 | `pending` |

## Feature coverage

- **service-health-check-probes** — O01, O02, E01 (operator + e2e surfaces).
  The in-process behaviour is covered by the four test tiers; these capture
  the operator-observable and qualitative slice those tiers under-serve.
- **udp-service-support** — O03 (deploy-accepted + udp-intent), E02 (the K1
  reverse-path-VIP-source proof). The in-process logic and the Tier-3 wire
  path are covered by the test tiers (notably the passing
  `reverse_nat_udp_e2e.rs`); these capture the operator-observable deploy
  half and the qualitative end-to-end #163-guard slice those tiers
  under-serve. E02 is the design-time `why` for the
  `reverse_nat_udp_e2e.rs` regression alarm (Stabilize doctrine).
- **built-in-ca** (GH #28) — E03 (full chain verifies under `openssl verify`,
  the walking-skeleton K1 proof), O04 (refuse-to-start on root-key decrypt
  failure with an actionable, cause-distinct error — K3 guardrail / Earned
  Trust), O05 (issuance observable as an `issued_certificates` audit row; no
  silent issuance), D01 (root key never plaintext at rest — K3 byte-scan). The
  in-process logic (CertSpec single-URI-SAN policy, SimCa DST determinism,
  AEAD envelope roundtrip, the `Ca` trait host/sim equivalence) is covered by
  the gated `integration-tests` Rust tiers (`ca_cert_spec_policy.rs`,
  `sim_ca_deterministic.rs`, `rcgen_ca_*.rs`, `ca_equivalence.rs`,
  `ca_boot_and_audit.rs`, `schema_evolution/{root_ca_key,issued_certificate_row}.rs`);
  these four expectations capture the operator/reviewer-observable slice those
  tiers under-serve. All `pending` **by design**: the CA is library-complete and
  proven by the gated tiers, but is intentionally not wired into the operator
  binary this phase (D-CA-4). Unblocked by **#215** (boot-side: wire `boot_ca`
  into `overdrive serve` → D01/O04) + **#35** (consumer-side: SVID issuance on
  alloc-start → E03/O05). Executed at SHA `2f4eccd4`; see
  `docs/evolution/2026-06-06-built-in-ca.md`.
- **canonical-workload-address-inbound-tproxy** (GH #241) — E04 (a mesh
  workload reachable at its canonical `workload_addr:service_port` over mTLS,
  end to end, the K1 round-trip proof). The in-process round-trip through the
  PRODUCTION-installed inbound nft-TPROXY rule is covered by the Tier-3 keystone
  `crates/overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs`
  (with a test PKI seam); E04 captures the black-box operator-observable slice
  that tier under-serves. `pending` **by design**: the black-box mesh-mTLS
  E-surface capture needs a converged full-system two-workload deploy with the
  PRODUCTION workload-identity CA proven black-box, provided by **#227** (the
  disposable full-system Lima VM EDD harness) on **#75** (the Image Factory OS
  image). Neither has landed, so E04 cannot be captured against the built binary
  yet.

## Adding an expectation

1. `mkdir verification/expectations/<SURFACE><NN>-<slug>/` with a `README.md`
   (scenario + `- Anchor:` lines + verification block + `Status: pending`).
2. Add an optional `runner.sh` that drives the **built** `overdrive` binary
   via the `od` helper (real commands; executed in Lima).
3. Add a row here.
4. Run `harness/run-expectation.sh <ID>`, review the evidence adversarially,
   then set the status in the expectation's `README.md`.
