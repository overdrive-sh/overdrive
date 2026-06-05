# O04 — Control plane refuses to start on root-key decrypt failure with an actionable error

**Surface:** O (operator CLI) · **KPI:** K3 (guardrail) · **Status:** `pending`

## Expectation

When the persisted root-CA key envelope cannot be decrypted at boot — wrong
KEK, or a corrupt/tampered ciphertext — the control plane **refuses to start**
with a **structured, actionable** error that names the cause, and it does
**not** silently re-mint a new root (a silent re-mint would orphan every
already-issued workload identity and break the trust hierarchy).

The error is qualitative, not just an exit code:

- It names the **cause** — *bad KEK* vs *corrupt/tampered envelope* are
  **distinct** messages (AES-GCM authentication distinguishes them).
- It is **actionable** — it points at the IntentStore path / the KEK source,
  not a cryptic panic or a bare backtrace.
- It emits `health.startup.refused` (per `development.md` § Intent =
  load-bearing — refuse to start, surface the structured signal).

This is the Earned-Trust probe contract (ADR-0063 D8 / § Earned Trust): *wire
→ probe → use*. The probe trial-decrypts the persisted envelope at composition
time, before the control plane accepts traffic.

- Anchor: S-02-06 (`boot_refuses_to_start_on_envelope_decrypt_failure_without_remint`)
- Anchor: S-02-07 (`boot_refuses_to_start_when_kek_absent_from_keyring`)
- Anchor: ADR-0063 D3 + § Earned Trust (refuse-to-start over silent re-mint)
- Anchor: docs/product/journeys/issue-workload-identity.yaml — error_paths step 1

## Verification

Precondition: the built-in CA boot path (DELIVER) — root-key persistence in
the IntentStore + the keyring/systemd-creds KEK provider + the Earned-Trust
probe. This expectation captures the **operator-observable** refuse-to-start
behaviour: the exact stderr an operator sees, and the absence of a re-minted
root.

Sub-claims:

1. With a tampered persisted envelope, `overdrive serve` refuses to start; the
   stderr names a **corrupt/tampered envelope** (actionable, not a panic).
2. With the wrong KEK, `overdrive serve` refuses to start; the stderr names a
   **wrong-KEK** cause — **distinct** from the tampered-envelope message.
3. With an absent keyring KEK (and no `OVERDRIVE_CA_KEK` dev opt-in),
   `overdrive serve` refuses to start **before any issuance** — no throwaway
   KEK is silently generated.
4. In every case the persisted root identity is **unchanged** (no silent
   re-mint): re-supplying the correct KEK afterward reuses the SAME root.

`satisfied` requires sub-claims 1–4 on a Lima run, reviewed adversarially for
"is the error actually actionable to an operator, or merely a non-zero exit?"
(Step 4 — don't outsource taste).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O04`. Not yet run —
the CA boot path lands in DELIVER. The gated integration tests in
`ca_boot_and_audit.rs` (S-02-06/07) prove the refuse-to-start in-tree; this
expectation captures the operator-visible stderr quality.
