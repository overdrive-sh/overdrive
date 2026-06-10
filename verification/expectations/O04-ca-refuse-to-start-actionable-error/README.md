# O04 — Control plane refuses to start on root-key decrypt failure with an actionable error

**Surface:** O (operator CLI) · **KPI:** K3 (guardrail) · **Status:**
`pending-recapture` (was `satisfied` at SHA `fc276c70`; the 2026-06-10
cause-taxonomy correction + the `TamperedEnvelope` → `EnvelopeAuthFailed`
rename invalidate the prior evidence's sub-claim labels — the crafter
re-captures against the corrected contract before re-asserting `satisfied`)

## Expectation

When the persisted root-CA key envelope cannot be decrypted at boot — wrong
KEK, or a corrupt/tampered ciphertext — the control plane **refuses to start**
with a **structured, actionable** error that names the cause, and it does
**not** silently re-mint a new root (a silent re-mint would orphan every
already-issued workload identity and break the trust hierarchy).

The error is qualitative, not just an exit code:

- It names the **cause** over the **three causes the system genuinely
  discriminates** (ADR-0063 D4 § "Honest decrypt-failure cause taxonomy",
  corrected 2026-06-10): **decrypt-auth-failure** vs **decode-malformed** vs
  **KEK-unavailable** are **pairwise-distinct** messages. The auth-failure
  message names **both** possibilities — wrong KEK material OR a
  tampered/corrupt envelope — because AES-GCM **cannot** distinguish them (both
  are one opaque authentication failure). The `kek_id`-mismatch cause
  (`CaError::WrongKek`) is the rotation/migration guard and is
  Phase-1-unreachable (hardcoded id); it is NOT one of the three operator
  causes here.
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

Sub-claims (corrected to the honest cause taxonomy, 2026-06-10):

1. With the **wrong KEK material** under the matching `kek_id` (the common
   operator case), `overdrive serve` refuses to start; the stderr names a
   **decrypt-auth-failure** cause (`CaError::EnvelopeAuthFailed`) that names
   **both** possibilities — wrong KEK material OR tampered/corrupt envelope
   (indistinguishable under AEAD) — actionable, not a panic.
2. With a **structurally corrupted** persisted envelope (bytes that no longer
   deserialize into a valid `RootCaKeyRecord`), `overdrive serve` refuses to
   start; the stderr names a **decode/Malformed** cause (fails before crypto)
   — **distinct cause class** from sub-claim 1. *(A tamper that keeps the
   record decodable is auth-failure, the SAME class as sub-claim 1 — to
   exercise a distinct cause the corruption must break the record structure.)*
3. With an absent keyring KEK (and no `OVERDRIVE_CA_KEK` dev opt-in),
   `overdrive serve` refuses to start **before any issuance** with a
   **KEK-unavailable** cause — no throwaway KEK is silently generated.
4. In every case the persisted root identity is **unchanged** (no silent
   re-mint): re-supplying the correct KEK afterward reuses the SAME root.

The cross-cause contract is that the three cause **classes**
(`{ EnvelopeAuthFailed, decode/Malformed, KekUnavailable }`) are
**pairwise-distinct** — asserted as cause-class distinctness, not bare
rendered-string inequality. `CaError::WrongKek` (id mismatch) is the
rotation-seam guard and is Phase-1-unreachable; it is NOT one of these three.

`satisfied` requires sub-claims 1–4 on a Lima run, reviewed adversarially for
"is the error actually actionable to an operator, or merely a non-zero exit?"
(Step 4 — don't outsource taste). **The current `evidence/` was captured
against the pre-correction sub-claim wording (the labels were swapped — the
"tampered" capture rendered decode/Malformed and the "wrong KEK" capture
rendered AES-GCM auth-failure); the crafter MUST re-capture against this
corrected contract + the renamed `EnvelopeAuthFailed` variant before
re-asserting `satisfied`.**

## Evidence

Executed through `harness/run-expectation.sh O04` at SHA `fc276c70` in Lima
(real kernel, real cgroup v2, real redb, production `SystemdCredsKeyring` KEK
provider), `executed_in_lima: true`, `runner_exit_code: 0`. The working tree
carries only untracked externals (`AGENTS.md`, the `deliver/` DES artifacts)
plus the just-written `evidence/` of this very capture, so the harness records
`working_tree_dirty: true`; no *tracked source* is modified (this re-capture
removed the prior dirty-tree asterisk where the runners themselves were
uncommitted). #215 wired `boot_ca` into `run_server`, so the refuse-to-start
paths are now reachable from `serve`.
The runner drives the BUILT `overdrive serve` binary BLACK-BOX (no `overdrive-*`
crate linked). Each boot runs under a FRESH kernel session keyring
(`keyctl session -`) so the production keyring KEK cache cannot leak across boots
and mask a refusal (the kernel-keyring-leak hazard).

Captured verdicts (all **PASS**, runner exit 0):

- Sub-claim 1 (tampered envelope) — refuses (exit 1); stderr: *"root CA key
  envelope decode failed; control-plane refusing to start … Malformed …"*,
  naming the redb path.
- Sub-claim 2 (wrong KEK) — refuses (exit 1); stderr: *"persisted root-key
  envelope failed to decrypt; control-plane refusing to start (no silent
  re-mint) … root-key envelope is corrupt or tampered (AES-GCM auth failed)"*,
  naming the redb path — DISTINCT from sub-claim 1.
- Sub-claim 3 (absent KEK) — refuses (exit 1); stderr: *"KEK unavailable at boot;
  control-plane refusing to start (no throwaway KEK minted) … no KEK registered
  for id `overdrive-ca-root`"*; and NO root-key envelope was persisted (no
  throwaway KEK).
- Pairwise-distinct — the three cause strings are pairwise distinct.
- Sub-claim 4 (no re-mint) — the persisted root cert PEM is byte-stable across
  the refused boot, and re-supplying the correct KEK adopts the SAME root.

The gated integration tests in `ca_boot_and_audit.rs` (S-02-06/07) plus the new
`serve_persistent_ca.rs` (S-OC-08a/b/c/d, S-OC-09, through the wired
`run_server`) prove the refuse-to-start in-tree; this expectation captures the
operator-visible stderr quality through the wired binary.

## Different-fox review

- **Reviewer:** `nw-software-crafter-reviewer` (Haiku) — a SEPARATE agent from
  the one that authored the implementation, the runner, and the evidence. The
  authoring agent did **not** self-stamp `satisfied`.
- **Verdict:** CONFIRMED.
- **SHA reviewed:** `5f4ca915` (evidence committed at `b2cc8e99`).
- **Date:** 2026-06-10.
- **Mode:** read-only over `evidence/run.log` + `evidence/verification.yaml`
  (the evidence, never the code that produced it), per
  `.claude/rules/verification.md` § "the different fox audit".

All four sub-claims demonstrated:

1. **Wrong-KEK / tampered-envelope / absent-KEK each refuse to start** with a
   non-zero exit and a **pairwise-distinct, actionable** stderr — each names
   the redb IntentStore path and the actual cause (malformed/decode vs AES-GCM
   auth-failure vs no-KEK-registered).
2. **No silent re-mint** — the persisted root cert hash `ef83f495…` is
   byte-stable across the refused → recovered boots; re-supplying the correct
   KEK adopts the SAME root.
3. **Fresh session keyring per boot** (`keyctl session -`) so the production
   keyring KEK cache cannot leak across boots and mask a refusal.
4. **Black-box** (no `overdrive-*` crate linked), `executed_in_lima: true`,
   `runner_exit_code: 0`.

Status set to `satisfied` by the orchestrator on the strength of the CONFIRMED
different-fox verdict above. No re-capture was needed — the evidence committed
at `b2cc8e99` is the reviewed artifact.
