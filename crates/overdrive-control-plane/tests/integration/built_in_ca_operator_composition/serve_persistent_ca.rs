//! Integration — `overdrive serve` persistent-CA boot composition per
//! `built-in-ca-operator-composition` Slice ② (folds GH #215 boot-side; closes
//! D-CA-4). DISTILL RED scaffolds.
//!
//! Layer 3 (real `RcgenCa` + real `SystemdCredsKeyring` KEK + real redb
//! `LocalIntentStore`; the `overdrive serve` CLI is the driving adapter, run as
//! a real subprocess in Lima per `.claude/rules/testing.md`). Per Mandate 11
//! these layer-3 sad paths are EXAMPLE-ONLY (one example per failure mode); no
//! PBT machinery.
//!
//! Settled design (feature-delta.md D-OC-4/5/6): `run_server` replaces the
//! ephemeral `RcgenCa::new` + `root()` + `issue_intermediate()` block with the
//! already-implemented, already-probing `boot_ca` + `bootstrap_node_intermediate`
//! path (KEK-resolve probe (a) → envelope decrypt-probe (b) → adopt-or-refuse).
//! `ControlPlaneError::CaBoot(#[from] CaBootError)` is the dedicated typed
//! variant so the distinct `CaError` cause (`WrongKek` vs `TamperedEnvelope`,
//! already-split Display) survives to the operator's stderr.
//!
//! EDD: S-OC-06/07 capture D01 (root key never plaintext at rest — on-disk
//! byte-scan); S-OC-08a/b/c/d + S-OC-09 capture O04 (refuse-to-start — one
//! scenario per cause: wrong-KEK / tampered-envelope / absent-KEK, plus a
//! pairwise-distinct-stderr contract — and no silent re-mint). The in-tree boot
//! tests in `ca_boot_and_audit.rs`
//! (S-02-06/07) already prove refuse-to-start at the `boot_ca` seam; these
//! scenarios prove the SAME behaviour through the WIRED `run_server` composition
//! root run as the built `overdrive serve` binary (the prior ephemeral path
//! probed nothing).
//!
//! RED scaffold convention (`.claude/rules/testing.md` § "What about
//! `#[ignore]`?"): these tests are `#[ignore]` — the blocker is the production
//! `run_server` CA-boot wiring (Slice ②) does not exist yet AND the runtime
//! surface (`overdrive serve` subprocess + real keyring) is Lima-only. DELIVER
//! removes `#[ignore]` and lands real assertions. The reason string names the
//! unblocking slice.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// S-OC-06 `@integration @real-io @adapter-integration @driving_port @slice-2
// @edd:D01` — on a CLEAN IntentStore with a resolvable KEK, `overdrive serve`
// FIRST boot generates a self-signed P-256 root + a node intermediate, persists
// the root as a KEK-sealed AES-256-GCM envelope in the IntentStore file, and
// reaches a serving state. EDD D01 sub-claims 1+2: the on-disk file carries the
// AEAD envelope fields (nonce + ciphertext + aead_tag) and ZERO plaintext
// root-key DER (byte-scan against the known first-boot key). Universe: the serve
// startup outcome + the on-disk IntentStore file contents.
#[test]
#[ignore = "blocked on Slice 2 — run_server boot_ca wiring (Lima; overdrive serve subprocess + real keyring)"]
fn serve_first_boot_generates_seals_and_persists_root() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-06 / overdrive serve first boot generates + \
         KEK-seals + persists a root, no plaintext key on disk -- EDD D01)"
    );
}

// S-OC-07 `@integration @real-io @adapter-integration @driving_port @slice-2
// @edd:D01` — a control plane that booted once and persisted a KEK-sealed root,
// restarted with the SAME KEK available, decrypts and ADOPTS the SAME root
// (identical root serial across the restart) and does NOT generate a new root.
// EDD D01 sub-claim 3: the on-disk file STILL contains no plaintext key bytes
// (the guardrail holds across the lifecycle). Universe: the root serial before
// vs after restart (identical) + the on-disk byte-scan.
#[test]
#[ignore = "blocked on Slice 2 — run_server boot_ca adopt-on-restart wiring (Lima)"]
fn serve_restart_adopts_same_root_no_remint() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-07 / overdrive serve restart adopts the SAME \
         root, identical serial, no re-mint, no plaintext on disk -- EDD D01)"
    );
}

// S-OC-08a `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start on the WRONG KEK: a persisted root whose envelope cannot be
// opened with the supplied KEK → CaError::WrongKek. The control plane does NOT
// begin serving and emits health.startup.refused; stderr names the wrong-KEK
// cause + the IntentStore path (not a bare panic/backtrace). EDD O04 sub-claim
// 1. NAMED example-based sad path (Mandate 11). Universe: the serve exit outcome
// + the wrong-KEK stderr cause string.
#[test]
#[ignore = "blocked on Slice 2 — ControlPlaneError::CaBoot wiring + boot_ca wrong-KEK refuse path (Lima)"]
fn serve_refuses_on_wrong_kek() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-08a / overdrive serve refuses to start on the \
         WRONG KEK -> CaError::WrongKek, cause-naming stderr, no serving -- EDD O04)"
    );
}

// S-OC-08b `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start on a TAMPERED envelope: a persisted root whose envelope has a
// mismatched AEAD tag → CaError::TamperedEnvelope. The control plane does NOT
// begin serving and emits health.startup.refused; stderr names the
// tampered-envelope cause + the IntentStore path. EDD O04 sub-claim 2. NAMED
// example-based sad path (Mandate 11). Universe: the serve exit outcome + the
// tampered-envelope stderr cause string.
#[test]
#[ignore = "blocked on Slice 2 — ControlPlaneError::CaBoot wiring + boot_ca tampered-envelope refuse path (Lima)"]
fn serve_refuses_on_tampered_envelope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-08b / overdrive serve refuses to start on a \
         TAMPERED envelope -> CaError::TamperedEnvelope, cause-naming stderr, no serving -- EDD O04)"
    );
}

// S-OC-08c `@integration @real-io @error @driving_port @slice-2 @edd:O04` —
// refuse-to-start when the KEK is ABSENT: NO KEK resolvable from the keyring →
// CaBootError::KekUnavailable, refused BEFORE any issuance, and NO throwaway KEK
// is generated. The control plane does NOT begin serving and emits
// health.startup.refused; stderr names the absent-KEK cause + the IntentStore
// path. EDD O04 sub-claim 3. NAMED example-based sad path (Mandate 11). Universe:
// the serve exit outcome + the absent-KEK stderr cause string + the absence of a
// generated throwaway KEK.
#[test]
#[ignore = "blocked on Slice 2 — ControlPlaneError::CaBoot wiring + boot_ca absent-KEK refuse path (Lima)"]
fn serve_refuses_on_absent_kek() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-08c / overdrive serve refuses to start when the \
         KEK is ABSENT -> CaBootError::KekUnavailable before any issuance, no throwaway KEK -- EDD O04)"
    );
}

// S-OC-08d `@integration @real-io @error @driving_port @slice-2 @edd:O04` — the
// three refusal causes render PAIRWISE-DISTINCT stderr: an operator can tell
// wrong-KEK, tampered-envelope, and absent-KEK apart from stderr alone. This
// pins the cross-cause triage value the original single-When scenario carried,
// without muddying the per-cause fail-for-right-reason triage. EDD O04
// sub-claims 1–3 (cross-cause contract). Example-based; compares the three
// stderr cause strings captured by S-OC-08a/b/c. Universe: the three captured
// stderr cause strings, compared for pairwise distinctness.
#[test]
#[ignore = "blocked on Slice 2 — boot_ca refuse paths + cause-distinct Display wiring (Lima)"]
fn serve_refusal_causes_are_pairwise_distinct() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-08d / the wrong-KEK / tampered-envelope / \
         absent-KEK refusal stderr messages are pairwise distinct -- EDD O04)"
    );
}

// S-OC-09 `@integration @real-io @error @driving_port @slice-2 @edd:O04` — a
// boot that REFUSED (wrong KEK or tampered envelope) does NOT silently re-mint:
// re-supplying the correct KEK and starting again adopts the SAME original root
// (identical serial), and no new root envelope was written during the refused
// boot. EDD O04 sub-claim 4 — the load-bearing guardrail (a silent re-mint would
// orphan every issued identity). Universe: the root serial after a
// refused-then-recovered boot (== original) + the IntentStore envelope
// (unchanged across the refused boot).
#[test]
#[ignore = "blocked on Slice 2 — boot_ca refuse-without-remint wiring (Lima)"]
fn refuse_to_start_does_not_remint_the_root() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-09 / a refused boot does NOT re-mint; \
         re-supplying the correct KEK adopts the SAME original root -- EDD O04)"
    );
}
