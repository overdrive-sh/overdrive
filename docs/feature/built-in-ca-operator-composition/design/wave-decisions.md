# DESIGN Wave Decisions — `built-in-ca-operator-composition`

**Mode:** propose (guide-mode rulings settled at dispatch) · **Paradigm:** OOP
Rust · **Scope:** Application · **Architect:** Morgan

## Key Decisions

| ID | Decision | Status |
|---|---|---|
| D-OC-1 | #40 near-expiry reissue = reconciler **action** (`Action::IssueSvid` rotate-correlation), NOT a workflow | Settled |
| D-OC-2 | Retire the `ROTATION_ENABLED` gate + `cert_rotation` workflow name; near-expiry emits `IssueSvid` unconditionally | Settled |
| D-OC-3 | Near-expiry threshold = ½ × `WORKLOAD_SVID_TTL` = **1800s** (verified: TTL is 3600s) | Settled |
| D-OC-4 | Wire `ca_boot::boot_ca` + `bootstrap_node_intermediate` into `run_server` (`lib.rs:1595`), replacing the ephemeral `RcgenCa` block | Settled |
| D-OC-5 | `ControlPlaneError::CaBoot(#[from] CaBootError)` — dedicated variant, never flatten to `Internal` | Settled |
| D-OC-6 | Restart = re-mint; #35's `ever_issued → IssueSvid` branch correct as-is (confirm only) | Confirmed |
| D-OC-7 | Additive `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary>`, latest-by-`issued_at` per running alloc | Settled |
| D-OC-8 | Un-skip the `near_expiry` mutation boundary (live target) | Settled |

## Architecture Summary

A composition + lifecycle-completion feature over the shipped `built-in-ca`
(ADR-0063) and `workload-identity-manager` (ADR-0067) subsystems. Three moves:

1. **Rotation as action.** `SvidLifecycle::reconcile`'s near-expiry branch flips
   from a gated `StartWorkflow(cert_rotation)` to an unconditional
   `Action::IssueSvid` (rotate correlation), reusing the existing variant. The
   internal SVID reissue is a single mint+swap — not a ≥2-external-step
   workflow.
2. **Persistent CA at boot.** `run_server` calls the already-implemented,
   already-probing `boot_ca` + `bootstrap_node_intermediate` (KEK probe → envelope
   decrypt-probe → adopt-or-refuse), replacing the ephemeral `RcgenCa` composition.
   Refuse-to-start surfaces a typed `CaBoot` error.
3. **Operator-visible current SVID.** The `alloc status` read aggregates the
   append-only `issued_certificates` audit and projects the current cert (latest
   row per running alloc by `issued_at`) into an additive response field.

Style: Hexagonal ports-and-adapters (established). No new subsystem, no new
dependency, no new public API surface beyond one additive wire struct.

## Reuse Analysis

See feature-delta.md § Reuse Analysis (HARD GATE). Summary: **11 REUSE AS-IS, 2
EXTEND (additive), 3 DELETE (single-cut), 0 CREATE-NEW** beyond the
`IssuedCertSummary` wire struct. `boot_ca`, the `IssueSvid` executor, the `Kek` /
`Ca` adapters, and the audit row are all reused as-is — only newly *called* or
*projected*.

## Tech Stack

No new dependencies. Reuses `ring`/`aws-lc-rs` (AES-GCM envelope), `rcgen`
(`RcgenCa`), `redb` (`IntentStore`), Corrosion/CR-SQLite (`ObservationStore`),
systemd-creds/kernel keyring (`SystemdCredsKeyring`) — all already in graph.

## Constraints

- **Single-node (Phase 2.6).** One node → one intermediate (multi-node = #36).
- **Leaf keys never at rest** (ADR-0063 D9) → restart = re-mint, driven by the
  audit-row `ever_issued` signal (ADR-0067 rev 5 D10).
- **No new public API surface** (CLAUDE.md "Implement to the design"): the rotate
  path reuses `Action::IssueSvid` UNCHANGED — no new field, flag, or variant.
- **Earned Trust:** the boot path must `probe-then-use`; the probes already exist
  in `boot_ca`, this feature wires them so they run at production boot.
- **`out-of-scope`:** `mint_ephemeral_ca()` (operator/CP-HTTPS CA, lib.rs:1237,
  D-CA-5/#81); whitepaper §18; ADR-0064/0065/0066.

## Upstream Changes (ADR/doc corrections — all 5)

| Target | Change |
|---|---|
| ADR-0067 (rev 6) | A5 reframe (rotation = permanent reconciler *action*, not throwaway sync-rotate); D8 + #40-boundary rewrite (emit `IssueSvid`, drop the wait-for-DNS-propagation workflow fiction); D1/D8 restart-re-mint re-validation note |
| ADR-0063 (dated amendment) | #215 wires `boot_ca`/`bootstrap_node_intermediate` into `run_server` (closes D-CA-4 "CA not wired into serve"); records `ControlPlaneError::CaBoot` + O04 cause-distinctness; D01/O04 pending→wired |
| `.claude/rules/workflows.md` § "Codebase precedent" | Correct "canonical first workflow = certificate rotation (#40) … wait for DNS propagation"; no first-party production workflow ships yet; candidate first = TBD (revocation-coupled rotation, Phase 5); internal SVID near-expiry reissue is a reconciler action |
| `docs/product/architecture/brief.md` | New `### Built-in CA operator composition` subsection under `## Application Architecture` |
| `.cargo/mutants.toml` | (DELIVER) remove the `"near_expiry"` `exclude_re` entry — the boundary is a live mutation target |

## DELIVER slices

① Rotation (core action flip + gate retire + un-skip mutation) · ② Boot-side
#215 (`boot_ca` wiring + `ControlPlaneError::CaBoot` + O04) → captures D01, O04 ·
③ Consumer-side #215, **two distinct surfaces with distinct proofs**:

- **`issued_certificates` field + CLI render → captures O05 ONLY.** Operator-legible
  metadata (`serial / spiffe_id / issuer_serial / not_after`, NO cert bytes, NO key).
  This render does NOT prove the chain verifies and MUST NOT be treated as
  satisfying E03.
- **E03 (full chain verifies) → captured SEPARATELY by the test-only `OD_E03_CA_DIR`
  PEM export** from `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs`
  plus `verification/expectations/E03-ca-full-chain-verifies/runner.sh`. **The
  current runner enforces ONLY sub-claims 1–2 then exits; Slice ③ MUST extend
  `runner.sh` to enforce ALL THREE E03 sub-claims before any `satisfied`:**
  (1) chain verifies — `openssl verify -CAfile root.pem -untrusted intermediate.pem
  svid.pem` → OK; (2) leaf profile — exactly one `spiffe://` URI SAN, `CA:FALSE`,
  critical `digitalSignature`; (3) the pathLen=0 negative anchor (S-03-05) — a chain
  where the pathLen=0 intermediate signs a *further CA* MUST FAIL `openssl verify`
  (pathLen *enforced*, not merely *set*). Sub-claim 3's source is named: under the
  same `OD_E03_CA_DIR` env-gate, export the further-CA chain from the existing test
  `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` and assert
  `openssl verify` FAILS (or capture that test's own failing-verification evidence).
  No production API surface and no operator verb mints/exports a chain (D-CA-4).
  See `feature-delta.md` § E03/O05 split for the full runner-wiring contract this
  mirrors.

**E03 is NOT satisfiable until the runner enforces sub-claims 1–3.** EDD per slice
via Lima `run-expectation.sh`, different-fox Haiku reviewer per expectation before
any `satisfied`. An agent must NOT mark E03 satisfied off the summary render (the PEM
`openssl verify` capture is the only E03 proof) AND must NOT mark E03 satisfied off
the present 2-check runner — the different-fox reviewer MUST reject E03 evidence that
omits the sub-claim-3 negative anchor.
