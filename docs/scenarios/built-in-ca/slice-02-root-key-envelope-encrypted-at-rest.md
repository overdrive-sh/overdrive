# Slice 02 — Root CA key envelope-encrypted at rest in the IntentStore

**Job**: J-SEC-001 | **Feature**: built-in-ca (GH #28) | **Story**: US-CA-02
**Walking skeleton**: part of the skeleton (makes the root *persistent*)

## Goal (one sentence)

Persist the Root CA across control-plane restarts by envelope-encrypting its
private key (AES-256-GCM DEK, passphrase-derived KEK) and storing only the
ciphertext in the IntentStore — never the plaintext key.

## IN scope

- Envelope encryption over `aws-lc-rs` AEAD (research Finding 8, Approach B):
  generate a data-encryption key (DEK), encrypt the root private key with it;
  derive a key-encryption-key (KEK) from an operator passphrase via
  scrypt/Argon2; store `{encrypted_dek, nonce, encrypted_private_key}`.
- Persist the encrypted blob in the IntentStore (redb), per whitepaper §4
  ("root CA key lives in the IntentStore, encrypted at rest"). State-layer:
  intent, not observation (CA material is deliberately never in the
  ObservationStore — whitepaper §4).
- `Ca` adapter gains: on boot, if an encrypted root exists → decrypt + reuse;
  else → generate (Slice 01) + encrypt + persist.
- Persist *inputs* not derived state (`.claude/rules/development.md`): the
  stored blob is the encrypted key material, not any derived/decoded form.

## OUT scope

- HSM / KMS / OS-keyring KEK source (research Finding 8 Approach C; Gap 2) →
  later phase. The KEK source is pluggable by construction; this slice uses
  passphrase-derived only.
- Root CA *rotation* (the SPIRE two-phase dual-bundle model, research Finding
  9) → that is a rotation workflow, GH #40 (depends on #39). This slice does
  single persistence + reuse, not rotation.
- Intermediate / leaf certs → Slices 03/04.

## Learning hypothesis

- **Disproves if it fails**: "the root survives a control-plane restart with
  its key protected by authenticated encryption, using only crypto already in
  the dependency graph (aws-lc-rs)." If envelope encryption + IntentStore
  persistence cannot round-trip the key, the persistent-CA premise (the whole
  point of superseding ADR-0010's ephemeral CA) is in question.
- **Confirms if it succeeds**: the platform has a durable, key-protected
  trust anchor; the ADR-0010 ephemeral-CA limitation is genuinely closed.

## Acceptance criteria

- [ ] First boot generates + persists; second boot decrypts + reuses the **same** root (same public key / same cert identity across restart).
- [ ] Only the envelope-encrypted blob is on disk — a test asserts the plaintext private key bytes do NOT appear in the IntentStore file. (Production-data AC.)
- [ ] AES-256-GCM authentication: a tampered ciphertext fails decryption with a distinct error from a wrong passphrase (`.claude/rules/development.md` § "Distinct failure modes get distinct error variants").
- [ ] Boot REFUSES to start on decryption failure with a structured, actionable error (cause + IntentStore path); it does NOT silently re-mint a new root (that would orphan existing identities). Surface `health.startup.refused`.
- [ ] DST: the persistence path is exercised against `LocalStore` with a fixture root key; reuse-on-reboot is asserted.

## Dependencies

- Slice 01 (the `Ca` trait + root generation).
- `aws-lc-rs` AEAD surface (already in graph). A passphrase-derivation crate
  (scrypt/argon2) — confirm one is in the workspace graph or add per
  `development.md` § Dependencies (workspace-pinned, standard crate).

## Effort estimate

~1 day (≤6h). Reference class: envelope encryption is a well-trodden pattern;
the redb persistence seam already exists (`LocalStore`).

## Pre-slice SPIKE

Optional ~1h confirm: which passphrase-KDF crate is already in the graph
(scrypt vs argon2) and the exact aws-lc-rs AEAD API shape. Low uncertainty —
fold into the slice unless the KDF crate must be added.

## Taste-test note

Production data (real encrypted key material, real redb roundtrip). Disproves
a real pre-commitment (persistent key-protected root). Not a scale-clone of
any other slice.
