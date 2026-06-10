# D01 — The root CA private key is never observable in plaintext at rest

**Surface:** D (dataplane / kernel- and disk-observable) · **KPI:** K3 (guardrail) · **Status:** `evidence-captured` (awaiting different-fox review)

## Expectation

Across the full lifecycle, the root CA private key is **never** present in
plaintext on disk. The IntentStore file that persists the root holds only the
AES-256-GCM-sealed envelope (`RootCaKeyRecordV1`: `kek_id`, HKDF `salt` +
`info`, GCM `nonce`, sealed `ciphertext`, `aead_tag`) — never the decoded key
DER. A byte-scan of the persisted record (and the IntentStore blob wrapping it)
finds **zero** plaintext private-key bytes.

The KEK that protects the root lives in **kernel space** (Linux kernel
keyring), not the process heap; systemd-creds (host-key/TPM-backed) delivers it
per boot. This expectation pins the at-rest guardrail (K3) — the disk-observable
half. The decrypt round-trip (the key is recoverable) is S-02-01; the *absence
of plaintext* is this expectation.

- Anchor: S-02-02 (`root_key_envelope_contains_no_plaintext_key_bytes`)
- Anchor: ADR-0063 D2 (root key = rkyv envelope in IntentStore) + D4 (HKDF → AES-256-GCM)
- Anchor: docs/feature/built-in-ca/feature-delta.md § Outcome KPIs — K3 (0 plaintext key bytes in the IntentStore file across the full lifecycle)

## Verification

Precondition: the built-in CA boot path (DELIVER) generates + envelope-encrypts
+ persists the root to the IntentStore. This expectation captures the
**disk-observable** guardrail: a byte-scan of the on-disk IntentStore for the
known plaintext key DER.

Sub-claims:

1. After first boot, the on-disk IntentStore file does **not** contain a
   plaintext private key — neither PEM-armored NOR unarmored raw DER (the
   strengthened byte-scan finds zero structural private-key markers; see the
   scan-rationale below).
2. The persisted record DOES contain the AEAD envelope fields (a `nonce` +
   `ciphertext` + `aead_tag` are present — i.e. the key is sealed, not absent).
3. After a restart that decrypts and reuses the root, the on-disk file STILL
   contains no plaintext key bytes (the guardrail holds across the lifecycle,
   not just at first write).

### Why a STRUCTURAL scan, not a known-key content scan

The first-boot root key is randomly generated (OsEntropy). This runner is
**black-box** (no `overdrive-*` crate linked) and so **cannot** know the
specific key's bytes a priori — and must not: a binary that leaked the
generated key to the runner would *itself* violate D01. A faithful "scan the
on-disk file for THIS key's plaintext" cannot be expressed black-box without
inventing serve-binary surface (a fixture-key boot flag / a test-only key
export), which `CLAUDE.md` § "Implement to the design — never invent API
surface" forbids.

The honest black-box proxy is to scan for the **structural markers any
plaintext P-256 private-key serialization must carry in the clear**, whatever
its random scalar:

- **PEM armor (ASCII):** `EC PRIVATE KEY` (SEC1) / `PRIVATE KEY` (PKCS#8).
- **Raw-DER OID byte runs (binary):**
  - `id-ecPublicKey` 1.2.840.10045.2.1 → `06 07 2A 86 48 CE 3D 02 01`
  - `prime256v1` 1.2.840.10045.3.1.7 → `06 08 2A 86 48 CE 3D 03 01 07`

  An unencrypted SEC1 `ECPrivateKey` carries the curve OID in its
  `parameters [0]` tag; a PKCS#8 `PrivateKeyInfo` carries both OIDs in its
  `privateKeyAlgorithm`. Either serialization emits at least one of these runs
  in the clear. A sealed AES-256-GCM envelope is high-entropy ciphertext: the
  chance either fixed 9/10-byte run appears by accident is ~2⁻⁷² / 2⁻⁸⁰ —
  zero in practice. So a **leaked plaintext key (armored OR raw-DER) WILL match;
  the sealed envelope WILL NOT**. This closes the raw-DER vacuous-pass gap the
  prior PEM-armor-only scan left open (an unarmored DER key carries no
  `-----BEGIN`).

  The persisted **public** root certificate also carries the curve OIDs in its
  `SubjectPublicKeyInfo` (a P-256 cert legitimately advertises its curve), so
  the scan **excises CERTIFICATE PEM blocks first** and runs the OID scan over
  the remainder — a leaked raw-DER private key lives in the redb VALUE bytes
  outside any CERTIFICATE block, so excision never hides it.

  The scan is proven non-vacuous: against real `openssl`-generated P-256 keys
  it DETECTS SEC1-PEM, PKCS#8-PEM, raw-SEC1-DER, and raw-PKCS#8-DER, while a
  public cert (post-excision) and 4 KiB of `os.urandom` ciphertext both scan
  CLEAN.

`satisfied` requires sub-claims 1–3 on a Lima run, reviewed adversarially for
"did the byte-scan actually run against the real on-disk file, or narrate it?"
(the different-fox audit reads only `evidence/`).

## Evidence

Executed through `harness/run-expectation.sh D01` at SHA `fc276c70` in Lima
(real kernel, real redb), `executed_in_lima: true`, `runner_exit_code: 0`. The
working tree carries only untracked externals (`AGENTS.md`, the `deliver/` DES
artifacts) plus the just-written `evidence/` of this very capture, so the
harness records `working_tree_dirty: true`; no *tracked source* is modified.
#215 wired `boot_ca` into `run_server`, so first boot now generates + KEK-seals
+ persists the root to the on-disk IntentStore. The runner drives the BUILT
`overdrive serve` binary BLACK-BOX (no `overdrive-*` crate linked) and runs the
strengthened structural byte-scan (armor + raw-DER OID runs, certs excised) over
the on-disk `intent.redb`:

- Sub-claim 1 — **PASS**: zero plaintext private-key markers on disk
  (`PEM-armor=0 DER-OID-runs=0`, after excising the public certificate PEM).
- Sub-claim 2 — **PASS**: the sealed root-key envelope is present
  (`key-envelope` marker, non-empty store).
- Sub-claim 3 — **PASS**: after a restart that decrypts + adopts the same root,
  STILL zero plaintext private-key markers (`PEM-armor=0 DER-OID-runs=0`).

The gated integration test
`rcgen_ca_root_key_envelope.rs::root_key_envelope_contains_no_plaintext_key_bytes`
(S-02-02) proves the no-plaintext invariant in-tree against the serialized
record; this expectation captures the on-disk IntentStore-file byte-scan through
the wired binary.

**Status gate**: this evidence is `evidence-captured`, NOT `satisfied`. Per
`.claude/rules/verification.md` § "the different fox audit", the authoring agent
MUST NOT self-stamp `satisfied`. A SEPARATE `*-reviewer` (Haiku) agent must read
`evidence/run.log` + `evidence/verification.yaml` adversarially and confirm
sub-claims 1–3 before the status is set to `satisfied`. The orchestrator
dispatches that review (the DELIVER subagent could not self-dispatch it).
