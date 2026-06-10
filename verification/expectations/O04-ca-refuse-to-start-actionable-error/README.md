# O04 — Control plane refuses to start on root-key decrypt failure with an actionable error

**Surface:** O (operator CLI) · **KPI:** K3 (guardrail) · **Status:**
`satisfied` (re-captured at SHA `87d53026` against the corrected ADR-0063 D4
cause taxonomy + the `TamperedEnvelope` → `EnvelopeAuthFailed` rename, and
CONFIRMED by a fresh different-fox audit — see § "Different-fox review")

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
(Step 4 — don't outsource taste). The evidence below was re-captured against
this corrected contract + the renamed `EnvelopeAuthFailed` variant and CONFIRMED
by a fresh different-fox audit (§ "Different-fox review").

## Evidence

Executed through `harness/run-expectation.sh O04` at SHA `87d53026` in Lima
(real kernel, real cgroup v2, real redb, production `SystemdCredsKeyring` KEK
provider), `executed_in_lima: true`, `runner_exit_code: 0`. The working tree
carries only untracked externals (`AGENTS.md`, the `deliver/` DES artifacts)
plus the just-written `evidence/` of this very capture, so the harness records
`working_tree_dirty: true`; no *tracked source* is modified (proven by the
retained `evidence/dirty-status.txt`). #215 wired `boot_ca` into `run_server`,
so the refuse-to-start paths are now reachable from `serve`. The runner drives
the BUILT `overdrive serve` binary BLACK-BOX (no `overdrive-*` crate linked).
Each boot runs under a FRESH kernel session keyring (`keyctl session -`) so the
production keyring KEK cache cannot leak across boots and mask a refusal (the
kernel-keyring-leak hazard).

This capture is against the corrected cause taxonomy (ADR-0063 D4) + the
`EnvelopeAuthFailed` rename. The runner classifies each refusal by a
cause-distinctive token — `AUTH` / `DECODE` / `KEK` — and asserts the three
classes pairwise-distinct (NOT bare rendered-string inequality).

Captured verdicts (all **PASS**, runner exit 0):

- Sub-claim 1 (wrong KEK **material**, matching id → class `AUTH`) — refuses
  (exit 1); stderr names the AES-GCM auth-failure cause AND **both**
  possibilities: *"failed AES-GCM authentication … the KEK material is wrong OR
  the envelope was tampered/corrupted (these are indistinguishable under
  AEAD)"*, naming the redb path.
- Sub-claim 2 (structurally corrupted envelope → class `DECODE`) — refuses
  (exit 1); stderr names a decode/Malformed cause (fails before crypto),
  naming the redb path — a **distinct cause class** from sub-claim 1, and the
  AES-GCM-auth-failure token is asserted ABSENT.
- Sub-claim 3 (absent KEK → class `KEK`) — refuses (exit 1); stderr names the
  KEK as unavailable; and NO root-key envelope was persisted (no throwaway KEK).
- Pairwise-distinct — the three cause **classes** (`AUTH` / `DECODE` / `KEK`)
  are pairwise distinct.
- Sub-claim 4 (no re-mint) — the persisted root cert PEM is byte-stable
  (`5fc3c01c…`) across the refused boot, and re-supplying the correct KEK
  adopts the SAME root.

The gated integration tests in `ca_boot_and_audit.rs` (S-02-06/07) plus
`serve_persistent_ca.rs` (S-OC-08a/b/c/d, S-OC-09, through the wired
`run_server`) prove the refuse-to-start in-tree (asserting the typed cause
class, not strings); this expectation captures the operator-visible stderr
quality through the wired binary.

## Different-fox review

The O04 expectation went through two adversarial cycles. The first
(SHA `5f4ca915`) CONFIRMED an earlier evidence set whose scenario labels were
later found to misname the cause classes — the step-02-03 code review surfaced
that AES-GCM cannot distinguish wrong-KEK-material from tampering, which drove
the ADR-0063 D4 correction + the `TamperedEnvelope` → `EnvelopeAuthFailed`
rename. The evidence was re-captured at SHA `87d53026` against the corrected
contract and re-audited by a fresh fox:

- **Reviewer:** `nw-software-crafter-reviewer` (Haiku) — a SEPARATE agent from
  the one that authored the implementation, the runner, and the evidence. The
  authoring agent did **not** self-stamp `satisfied`.
- **Verdict:** CONFIRMED — no defects.
- **SHA reviewed:** `87d53026`.
- **Date:** 2026-06-10.
- **Mode:** read-only over `evidence/` (run.log + verification.yaml +
  dirty-status.txt) + the `runner.sh` methodology — never the code that
  produced it — per `.claude/rules/verification.md` § "the different fox audit".

Confirmed against the corrected contract:

1. **Auth-failure message is honest** — it names *both* "the KEK material is
   wrong OR … tampered/corrupted (indistinguishable under AEAD)", not
   specifically "tampered". (This is the defect the code review caught.)
2. **Distinctness is by cause class, not string** — the runner classifies each
   refusal as `AUTH` / `DECODE` / `KEK` via cause-distinctive tokens and
   asserts the three classes differ; it does NOT `assert_ne!` on whole log
   lines.
3. **Decode-malformed is a genuinely distinct class** (fails at deserialize,
   before crypto); the auth-failure token is asserted absent from it.
4. **Absent-KEK refuses before issuance**, no throwaway KEK persisted.
5. **No silent re-mint** — persisted root cert hash `5fc3c01c…` byte-stable
   across refused → recovered boots.
6. **Fresh session keyring per boot**; **black-box**; `executed_in_lima: true`,
   exit 0; dirty tree is externals-only.

Status set to `satisfied` by the orchestrator on the strength of the CONFIRMED
fresh-fox verdict above, against the re-captured `87d53026` evidence.
