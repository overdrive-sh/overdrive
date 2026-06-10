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

1. After first boot, the on-disk IntentStore file does **not** contain the
   plaintext root-key DER (byte-scan finds zero occurrences of the known key
   bytes / the DER PKCS#8 marker for the generated key).
2. The persisted record DOES contain the AEAD envelope fields (a `nonce` +
   `ciphertext` + `aead_tag` are present — i.e. the key is sealed, not absent).
3. After a restart that decrypts and reuses the root, the on-disk file STILL
   contains no plaintext key bytes (the guardrail holds across the lifecycle,
   not just at first write).

`satisfied` requires sub-claims 1–3 on a Lima run, reviewed adversarially for
"did the byte-scan actually run against the real on-disk file, or narrate it?"
(the different-fox audit reads only `evidence/`). Note: a true byte-scan needs
the *known* plaintext key to scan for — the runner derives it from the same
fixture/first-boot key the test uses, not a guess.

## Evidence

Executed through `harness/run-expectation.sh D01` at SHA `aaaaa0cd` in Lima
(real kernel, real redb), `executed_in_lima: true`, `runner_exit_code: 0`. #215
wired `boot_ca` into `run_server`, so first boot now generates + KEK-seals +
persists the root to the on-disk IntentStore. The runner drives the BUILT
`overdrive serve` binary BLACK-BOX (no `overdrive-*` crate linked) and byte-scans
the on-disk `intent.redb`:

- Sub-claim 1 — **PASS**: zero plaintext PRIVATE KEY PEM markers on disk
  (`BEGIN=2 PRIVKEY=0`; the 2 BEGIN blocks are PUBLIC certificate PEM, not the
  private key).
- Sub-claim 2 — **PASS**: the sealed root-key envelope is present
  (`key-envelope` marker, non-empty store).
- Sub-claim 3 — **PASS**: after a restart that decrypts + adopts the same root,
  STILL zero plaintext PRIVATE KEY PEM (`BEGIN=2 PRIVKEY=0`).

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
