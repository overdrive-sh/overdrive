# Review: Built-in CA Operator Composition (Step 03-02)

**Reviewer:** codex_nw_review
**Review Date:** 2026-06-11
**Step:** `03-02` - `overdrive alloc status` CLI render of the issued-certificates section
**Reviewed State:** `HEAD` (`6e66f3a5`, including the follow-up live-render fixes after `4658318b`)
**Verdict:** **REVISIONS NEEDED**

## Findings

### 1. High - Equal `issued_at` ties can surface a stale certificate as current

The alloc-status projection selects the current certificate with:

`crates/overdrive-control-plane/src/handlers.rs:1033-1037`

```rust
issued_cert_rows
    .iter()
    .filter(|c| c.spiffe_id == spiffe)
    .max_by_key(|c| c.issued_at)
```

This only defines "latest" when `issued_at` is strictly ordered. Equal timestamps are reachable because issuance reads from an injected `Clock`, and existing tests already issue twice with the same fixed clock (`ca_boot_and_audit.rs:926-937`). The observation stores return issued-certificate rows keyed by serial (`LocalObservationStore` table key is `incoming.serial`; `SimObservationStore` stores in a `BTreeMap` by serial), not by issuance order. When two rows for the same SPIFFE ID share the same `issued_at`, the selected row becomes dependent on serial ordering / iterator tie behavior rather than the leaf currently held after the second issue.

Impact: S-OC-12's "post-restart serial change reads as the current cert rather than an anomaly" can be false under deterministic or same-tick issuance. An operator can see the previous cert serial even though `IdentityMgr` holds the replacement.

Recommendation: make the current-cert projection deterministic for same-time reissue. Options include recording a monotonic issuance sequence/write order in the audit row, enforcing unique `issued_at` per issuance at the issuance seam, or projecting the summary from the currently held SVID serial where that is the contract. Add a regression with two rows for the same SPIFFE ID and identical `issued_at` that proves the replacement serial is rendered as current.

### 2. High - S-OC-12 is not actually exercised by the new tests

The roadmap requires S-OC-12 to cover "multiple issued-certificate rows over time (first issue + a re-mint)" and prove the render shows exactly the latest-by-`issued_at` row, not history.

The active 03-02 tests do not create that condition:

- `crates/overdrive-cli/tests/integration/alloc_status.rs:750-772` constructs exactly one `IssuedCertSummary`.
- `crates/overdrive-cli/tests/acceptance/render_alloc_status.rs:541-557` also constructs exactly one summary.
- `IssuedCertSummary` intentionally does not carry `issued_at`, so render-layer-only tests cannot prove latest-by-`issued_at` selection.
- The misplaced control-plane scaffold was deleted, but no replacement handler/server test drives `issued_certificate_rows()` with two rows for the same alloc and verifies only the latest summary reaches the CLI render.

This means a regression that returns all matching audit rows as summaries, or returns the older row, can still pass the 03-02 render tests as long as the manually constructed response contains one summary.

Recommendation: add a focused control-plane handler/projection test or an in-process integration test that seeds two `IssuedCertificateRow`s for the same running alloc with different `issued_at` values, calls the alloc-status read path, and renders the live CLI output. Assert the older serial is absent, the newer serial is present once, and the PEM/private-key forbidden tokens are absent.

## Scope Reviewed

Primary step-03-02 files reviewed:

- `crates/overdrive-cli/src/render.rs`
- `crates/overdrive-cli/tests/integration/alloc_status.rs`
- `crates/overdrive-cli/tests/acceptance/render_alloc_status.rs`
- `crates/overdrive-cli/tests/acceptance/render_pure_fns.rs`
- `crates/overdrive-control-plane/tests/integration.rs`
- deleted misplaced scaffold: `crates/overdrive-control-plane/tests/integration/built_in_ca_operator_composition/alloc_status_issued_certificates.rs`

Supporting server projection reviewed because the 03-02 acceptance criteria depend on it:

- `crates/overdrive-control-plane/src/handlers.rs`
- `crates/overdrive-control-plane/src/api.rs`

## Criteria Conformance

The live CLI render now reads `out.snapshot.issued_certificates` and renders the section from the single live `render::alloc_status` path. This corrects the earlier `alloc_status_kind_aware` dead-path problem.

The section is presence-guarded and prints only the four summary fields:

- `serial`
- `spiffe_id`
- `issuer_serial`
- `not_after`

No certificate bytes or private key fields are present in `IssuedCertSummary`, and the renderer does not reconstruct cert material.

The missing piece is the load-bearing "latest, not history" behavior. That behavior lives in server projection, not rendering, and is not sufficiently tested by the current 03-02 test shape.

## Test Quality

The render tests are useful for proving the live path renders provided summary facts and omits the section when empty. They are not sufficient as S-OC-12 acceptance coverage because they manually construct the already-projected `Vec<IssuedCertSummary>` and provide only one row.

The current S-OC-12 test would still pass if the server projected the wrong row. That is a completeness gap, not just a naming mismatch.

## Verification

Per user instruction, I did not run acceptance gates, nextest suites, mutation testing, Lima runners, or verification runners.

Review was static/read-only only: roadmap, execution log, git diff, current source, current tests, and relevant observation-store/issuance code.

## Decision

**REVISIONS NEEDED** for step `03-02`.

The CLI render path itself is close, but the step's core "current cert, latest not history" guarantee is either ambiguous under equal timestamps or untested for the multi-row condition the roadmap explicitly requires.
