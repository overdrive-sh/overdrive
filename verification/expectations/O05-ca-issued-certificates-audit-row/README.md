# O05 — Every issuance is observable as an `issued_certificates` audit row

**Surface:** O (operator CLI) · **KPI:** K1 (auditability supports the trust story) · **Status:** `pending`

## Expectation

Every workload SVID the platform issues writes an `issued_certificates`
**observation** row — the internal-CT-equivalent audit surface — readable via
the existing `overdrive alloc status --job <id>` observation path. The row
records `serial`, `spiffe_id`, `issuer_serial`, `not_before`, `not_after`,
`node_id`, `issued_at`, and the `serial` / `spiffe_id` / `issuer_serial` match
the minted certificate.

**Issuance is never silent**: an issuance whose audit row cannot be written
surfaces an error rather than handing out an unaudited certificate (issuance +
audit are observable together). The CA *material* is intent (persisted in the
IntentStore, never gossiped); the *record of what was issued* is observation
(gossiped when multi-node #36 lands).

- Anchor: S-05-03 (`issuance_writes_issued_certificates_row_matching_the_minted_cert`)
- Anchor: S-05-04 (`issuance_that_cannot_write_audit_row_surfaces_an_error` — no silent issuance)
- Anchor: ADR-0063 D6 (`issued_certificates` ObservationStore audit row)
- Anchor: docs/product/journeys/issue-workload-identity.yaml — step 4 ("issuance is auditable")

## Verification

Precondition: the built-in CA issuance path (DELIVER) writes the
`issued_certificates` observation row on alloc-start, and the existing
`alloc status` path surfaces it. This expectation captures the
**operator-observable** audit surface.

Sub-claims:

1. After the platform issues an SVID for a deployed workload,
   `overdrive alloc status --job <id>` surfaces the `issued_certificates`
   record (serial / spiffe_id / issuer visible).
2. The surfaced `serial` and `spiffe_id` match the minted certificate
   (cross-checked against `openssl x509 -in svid.pem -noout -serial -ext subjectAltName`).
3. (Negative anchor, from S-05-04) when the audit-row write is forced to fail,
   the issuance surfaces an error and no unaudited certificate is handed out.

`satisfied` requires sub-claims 1–3 on a Lima run, reviewed adversarially for
"is the audit row actually legible / does the operator see what was issued?"
(Step 4 — don't outsource taste).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O05`. Not yet run —
the CA issuance + audit-row path lands in DELIVER. The gated integration tests
in `ca_boot_and_audit.rs` (S-05-03/04) prove the row write + no-silent-issuance
in-tree; this expectation captures the operator-visible read surface.
