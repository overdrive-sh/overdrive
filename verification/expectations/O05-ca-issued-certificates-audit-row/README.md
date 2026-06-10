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

Executed through `harness/run-expectation.sh O05` at SHA `c5702a13`
(working tree dirty — the refined runner is in `evidence/dirty-diff.patch`),
`executed_in_lima: true`, runner exit 0, and **self-reports `pending`**.

The `evidence/preflight_cluster.out` capture shows the honest blocker: the O05
runner reads an **already-running deployment**, and no `overdrive serve` control
plane is reachable in the harness (`od cluster status` → `failed to reach
overdrive control plane … could not connect to server`, exit 1). The runner does
not stand up a persistent `serve` lifecycle itself — bringing up `serve` →
`deploy` → converge-to-Running → SVID-issuance-on-alloc-start → `alloc status`
is a multi-component live flow that a single harness runner invocation cannot
orchestrate.

What this slice DID land for O05:

- The **live render surface is correct and matched.** The runner now greps the
  actual operator render — heading `Issued certificates:` with the four
  audit-row facts `serial:` / `spiffe_id:` / `issuer_serial:` / `not_after:`
  (`render::render_issued_certificates_section`,
  `crates/overdrive-cli/src/render.rs`, wired into the live `alloc status` path
  by deps 03-01/03-02) — instead of the prior loose case-insensitive grep.
- A **negative no-leak check** was added: the render is metadata-only, so a
  `BEGIN CERTIFICATE` / `BEGIN … PRIVATE KEY` block in `alloc status` would
  FAIL the runner (the audit row persists only facts; the workload holds no
  SVID material — the kernel does mTLS, per CLAUDE.md's workload-identity
  model).

So when a deployment that issues an SVID IS reachable, the runner is ready to
assert sub-claims 1+2 over the real render. The in-tree gated tests in
`ca_boot_and_audit.rs` (S-05-03/04) already prove the row write +
no-silent-issuance forever; this expectation captures the operator-visible read
surface, which is **blocked on a live end-to-end deployment the harness cannot
stand up in one runner invocation**.

**Status candidate: `pending` (honest).** The render surface is correct, but
the live deployment that would produce an `issued_certificates` row is not
reachable here — see the surfaced blocker in the step handoff. Not narrated: the
`pending` reflects a real `executed_in_lima: true` capture whose preflight
failed against a non-running control plane, not a believed outcome.
