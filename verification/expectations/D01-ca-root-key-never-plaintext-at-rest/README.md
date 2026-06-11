# D01 — The root CA private key is never observable in plaintext at rest

**Surface:** D (dataplane / kernel- and disk-observable) · **KPI:** K3 (guardrail) · **Status:** `satisfied` (re-captured at SHA `ed16f60e` after the restart sub-claim was STRENGTHENED in response to an adversarial-review High finding: the restart now asserts a **serving witness** + a **stable root-identity witness** with BOTH pre- and post-restart serials printed for audit, not just a plaintext re-scan — a refused restart that left disk unchanged previously satisfied the runner. A fresh 3-fox different-fox audit CONFIRMED the strengthened evidence — see § "Different-fox review")

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
3. The restart actually **reached serving** — `serve::run` logs `control plane
   listening` only after the listener binds, and a refuse-to-start returns `Err`
   before it, so the marker's presence proves the second boot decrypted the
   sealed envelope and adopted the root rather than refusing. (Without this, a
   restart that REFUSED would leave the disk byte-identical and pass sub-claim 5
   vacuously — the adversarial-review High finding.)
4. The restart adopted the **SAME root**, not a re-mint — the persisted
   CERTIFICATE serial(s) are byte-stable across the restart (S-OC-07 "identical
   serial across restart"). A silent re-mint would replace the root keypair with
   a new cert carrying a new serial.
5. After the decrypt-and-adopt restart, the on-disk file STILL contains no
   plaintext key bytes (the guardrail holds across the lifecycle, not just at
   first write).

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

  The scan is proven non-vacuous **inline in the runner** (so `run.log` shows
  it, not narrated out-of-band): the runner generates a throwaway P-256 key in
  its temp space (`openssl`, Lima) in all four serializations and runs the SAME
  scan function over each. Positive controls — it DETECTS SEC1-PEM, PKCS#8-PEM,
  raw-SEC1-DER, and raw-PKCS#8-DER (each a `[selftest PASS] detects …` line).
  Negative controls — it stays CLEAN on (i) a real P-256 public cert AFTER the
  cert-excision step (proving excision does not blind the scan to a key) and
  (ii) 4 KiB of `openssl rand` ciphertext-sized bytes (each a
  `[selftest PASS] clean on …` line). The self-test is a HARD GATE: any
  positive-control miss or negative-control false-match fails the runner
  (non-zero exit) before the production scan ever runs. The throwaway key never
  touches the real IntentStore.

`satisfied` requires sub-claims 1–5 on a Lima run, reviewed adversarially for
"did the byte-scan actually run against the real on-disk file, or narrate it?"
**and** "did the restart actually reach serving + adopt the same root, or could
a refused restart that left disk untouched have passed?" (the different-fox
audit reads only `evidence/`).

## Evidence

Executed through `harness/run-expectation.sh D01` in Lima (real kernel, real
redb), `executed_in_lima: true`, `runner_exit_code: 0`. The working tree at
capture carries only the in-flight verification work for this step (this
expectation's `runner.sh` + `README.md`, the O04 `README.md`, and the
just-written `evidence/`) plus untracked externals (`AGENTS.md`, the `deliver/`
DES artifacts) — **no tracked `.rs`/production source is modified** (proven by
the retained `evidence/dirty-status.txt` artifact, NOT narrated). #215 wired
`boot_ca` into `run_server`, so first boot now generates + KEK-seals + persists
the root to the on-disk IntentStore. The runner drives the BUILT `overdrive
serve` binary BLACK-BOX (no `overdrive-*` crate linked).

First, an **inline non-vacuity self-test** runs (printed to `run.log`) — it
generates a throwaway P-256 key in temp space and proves the byte-scan DETECTS
all four plaintext serializations and stays CLEAN on benign inputs; it is a
hard gate that aborts the runner on any miss/false-match. Then the SAME
strengthened structural byte-scan (armor + raw-DER OID runs, certs excised)
runs over the real on-disk `intent.redb`:

- Self-test — **PASS**: `[selftest PASS] detects SEC1-PEM / PKCS8-PEM /
  raw-SEC1-DER / raw-PKCS8-DER` (4/4 positive controls) and `[selftest PASS]
  clean on public-cert-post-excision / random-ciphertext-sized-bytes` (2/2
  negative controls) → `scan proven non-vacuous`.
- First boot — **PASS**: persisted the IntentStore AND reached serving
  (`control plane listening`).
- Sub-claim 1 — **PASS**: zero plaintext private-key markers on disk
  (`PEM-armor=0 DER-OID-runs=0`, after excising the public certificate PEM).
- Sub-claim 2 — **PASS**: the sealed root-key envelope is present
  (`key-envelope` marker, non-empty store).
- Sub-claim 3 — **PASS**: the restart decrypted + adopted the root and reached
  serving (`control plane listening`, `rc=0`).
- Sub-claim 4 — **PASS**: the restart adopted the SAME root — the persisted cert
  serial(s) are printed BOTH before and after the restart so the equality is
  visible in the artifact (not asserted only by the badge): `pre-restart cert
  serial(s): serial=62E43C7B6CC4D56D5D30F86DD1AD736B
  serial=F1D73D9BA6019331C3DDE29D69C05CD4` and `post-restart cert serial(s):`
  the SAME two values — byte-identical (the second serial is the node
  intermediate). A re-mint would change them. (The concrete serials are random
  per capture — each boot mints a fresh root — so they differ run-to-run; the
  load-bearing fact is pre == post.)
- Sub-claim 5 — **PASS**: after the decrypt-and-adopt restart, STILL zero
  plaintext private-key markers (`PEM-armor=0 DER-OID-runs=0`).

The gated integration test
`rcgen_ca_root_key_envelope.rs::root_key_envelope_contains_no_plaintext_key_bytes`
(S-02-02) proves the no-plaintext invariant in-tree against the serialized
record; this expectation captures the on-disk IntentStore-file byte-scan through
the wired binary.

### Different-fox review

Per `.claude/rules/verification.md` § "the different fox audit", the authoring
agent did NOT self-stamp. The orchestrator dispatched independent adversarial
`nw-software-crafter-reviewer` (Haiku) foxes reading only `evidence/`:

- Fox 1 — **REFUTED**: the byte-scan caught only PEM armor, so an unarmored
  raw-DER key would pass vacuously; also captured against a dirty tree.
- Fox 2 — **REFUTED**: raw-DER OID scan accepted as sound, but non-vacuity was
  asserted in prose (not demonstrated in `run.log`) and the dirty-tree
  cleanliness lacked a proof artifact.
- Fox 3 — **CONFIRMED** (SHA `bc43082c`, 2026-06-10, no novel defects): the
  inline self-test demonstrates non-vacuity (4/4 plaintext encodings detected,
  2/2 benign inputs clean, hard gate before the production scan); sub-claims
  1–3 PASS with zero markers on first boot and restart; `dirty-status.txt`
  proves externals-only with no modified tracked source; `executed_in_lima:
  true`, exit 0.

- Fox 4 — **CONFIRMED** (SHA `87d53026`, 2026-06-10, no novel defects): the
  evidence was re-captured at `87d53026` alongside the O04 cause-taxonomy
  correction (the D01 byte-scan contract itself is unchanged). A fresh fox
  re-verified the inline non-vacuity self-test (4/4 detected, 2/2 benign clean,
  hard gate), zero plaintext markers across first boot + restart, the
  sealed-envelope-present check, and externals-only dirty tree; Fox 1 & 2's
  cured grounds stay cured.

Fox 3 + Fox 4 confirmed the byte-scan contract (sub-claims 1–3 of the original
runner). A LATER adversarial review (2026-06-10) raised a **High** finding the
prior foxes did not cover: the restart section only re-scanned the file for
plaintext and never asserted the second boot reached serving or adopted the same
root, so a restart that REFUSED (leaving disk unchanged) would still satisfy the
runner — even though S-OC-07 (roadmap AC) requires the restart to decrypt + adopt
the SAME root. The runner was strengthened (the restart now asserts a serving
witness + a stable cert-serial identity witness; old sub-claim 3 → sub-claim 5)
and the evidence re-captured at `ba8ddd51` (all of self-test, first-boot-serving,
sub-claims 1–5 PASS, exit 0).

Because the runner materially changed, the Fox 3/Fox 4 confirmations no longer
covered the live evidence. A FRESH 3-fox audit was dispatched against the
strengthened evidence (foxes read only `evidence/` + the roadmap anchor, never
the runner or production code):

- Fox 5 — **CONFIRMED** (SHA `ed16f60e`, 2026-06-10): all five sub-claims back
  by concrete run.log lines; serving witness present (rc=0); cert serials
  byte-stable across restart; non-vacuity self-test 4/4 + 2/2; externals-only
  dirty tree.
- Fox 6 — **CONFIRMED**: same, with emphasis that the inline self-test is a hard
  gate and the restart witness cannot hide a refused restart.
- Fox 7 — initially **REFUTED**: sub-claim 4 printed only the POST-restart
  serials + a `[PASS]` badge; the PRE-restart serials were never displayed, so a
  fox reading only the evidence could not independently confirm pre == post (a
  re-mint that swapped serials could pass undetected). The runner was fixed to
  print BOTH the pre-restart and post-restart serial lines unconditionally; the
  evidence was re-captured. Fox 7 re-audited and **CONFIRMED** — it compared the
  two printed serial lines character-by-character (`62E4…736B` + `F1D7…5CD4`,
  byte-identical) and explicitly recorded the original gap as CLOSED.

Status set to `satisfied` on the Fox 5 + Fox 6 + Fox 7 confirmations. The
authoring agent did NOT self-stamp — the verdict is the independent foxes'.
