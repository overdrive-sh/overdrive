//! Integration — CA boot wiring, refuse-to-start, persistence, audit row
//! (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 3 (gated `integration-tests`, runs via Lima — exercises the real
//! `IntentStore` (`LocalStore` over redb), the `ObservationStore`, and the
//! Earned-Trust composition-root probes). These prove the CONSUMER wiring:
//! how the control-plane boot path generates-or-loads the root, refuses to
//! start on probe failure, persists across restart, and writes the audit
//! row on issuance.
//!
//! Per Mandate 11 these layer-3 tests are EXAMPLE-ONLY; each failure mode
//! from the SSOT journey `error_paths` and ADR-0063 § "Earned Trust" gets
//! exactly one named example.
//!
//! Earned-Trust probe contract (ADR-0063 D8): wire -> probe -> use. On boot
//! the CA adapter probes (a) KEK present in keyring, (b) persisted envelope
//! decrypts, (c) systemd-creds credential present — BEFORE the control
//! plane accepts traffic; a probe failure refuses startup with
//! `health.startup.refused`, never a silent fallback / silent re-mint.
//!
//! Scenarios trace to: US-CA-02 (persistence reuse, refuse-to-start),
//! US-CA-03 (intermediate signing failure), US-CA-05 (audit row, no silent
//! issuance), SSOT journey `error_paths` steps 1-3.
//! Tags: `@real-io` `@adapter-integration` `@S-NN` · `+ @error` (failure paths).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt CA
//! wiring. DELIVER replaces with real boot/issuance assertions.

// ---------------------------------------------------------------------------
// US-CA-02 / S-02 — Persistent root reused across restart (happy path)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — persistence: first boot
/// generates + envelope-encrypts + persists the root to the `IntentStore`;
/// second boot (same KEK) decrypts and REUSES the same root identity (same
/// public key / same cert). This is what supersedes ADR-0010's ephemerality.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_ca_is_reused_across_control_plane_restart() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / first boot persists the root; second boot \
         decrypts and reuses the SAME root identity (same public key) across restart)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-02 / S-02 — Refuse-to-start on decrypt failure (Earned Trust)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — SSOT journey
/// `error_paths` step 1: a tampered/undecryptable persisted envelope makes
/// the boot-time Earned-Trust probe FAIL; the control plane REFUSES to
/// start with a typed `CaError` + `health.startup.refused`, and does NOT
/// silently re-mint a new root (which would orphan every issued identity).
#[test]
#[should_panic(expected = "RED scaffold")]
fn boot_refuses_to_start_on_envelope_decrypt_failure_without_remint() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / undecryptable persisted envelope -> \
         Earned-Trust probe fails -> control plane refuses to start (health.startup.refused), \
         no silent re-mint)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` `@error` — Earned-Trust KEK
/// probe: an absent/empty keyring KEK (and no dev `OVERDRIVE_CA_KEK`
/// opt-in) refuses startup BEFORE any issuance, rather than panicking
/// mid-issuance or silently generating a throwaway KEK (which would make
/// at-rest encryption meaningless).
#[test]
#[should_panic(expected = "RED scaffold")]
fn boot_refuses_to_start_when_kek_absent_from_keyring() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / absent keyring KEK (no OVERDRIVE_CA_KEK \
         opt-in) -> refuse to start before any issuance, no silent KEK generation)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-03 / S-03 — Intermediate signing failure fails loudly
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-03` `@error` — SSOT journey
/// `error_paths` step 2: when the root key is unavailable at node bootstrap
/// (decrypt failed upstream), `issue_intermediate` surfaces a typed
/// `CaError`; node bootstrap fails loudly rather than running workloads it
/// cannot issue identities for (no half-provisioned state).
#[test]
#[should_panic(expected = "RED scaffold")]
fn intermediate_signing_failure_fails_node_bootstrap_loudly() {
    panic!(
        "Not yet implemented -- RED scaffold (S-03 / root key unavailable at node bootstrap -> \
         issue_intermediate surfaces a typed CaError -> node bootstrap fails loudly, no \
         half-provisioned state)"
    );
}

// ---------------------------------------------------------------------------
// US-CA-05 / S-05 — Audit row written; no silent issuance; re-issue
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-05` — every issuance writes an
/// `issued_certificates` observation row; a test reads it back via the
/// `ObservationStore` and asserts serial + `spiffe_id` + `issuer_serial` match
/// the minted cert (the internal-CT-equivalent audit surface, readable via
/// the existing `alloc status` path).
#[test]
#[should_panic(expected = "RED scaffold")]
fn issuance_writes_issued_certificates_row_matching_the_minted_cert() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / issuance writes an issued_certificates row; \
         read-back asserts serial + spiffe_id + issuer_serial match the minted cert)"
    );
}

/// `@real-io` `@adapter-integration` `@S-05` `@error` — no silent issuance
/// (US-CA-05 AC + SSOT journey): an issuance whose `issued_certificates`
/// audit row cannot be written surfaces a `CaError` rather than handing out
/// an unaudited certificate (issuance + audit are observable together).
#[test]
#[should_panic(expected = "RED scaffold")]
fn issuance_that_cannot_write_audit_row_surfaces_an_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / an issuance whose audit row cannot be written \
         surfaces a CaError, never hands out an unaudited certificate)"
    );
}

/// `@real-io` `@adapter-integration` `@S-05` — re-issue without restart: the
/// platform re-issues a fresh SVID for an existing `SpiffeId`; a new leaf
/// (distinct serial, new validity window) is produced and the control plane
/// is NOT restarted — the re-issue mechanism the #40 rotation workflow will
/// later drive on a schedule.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_is_reissued_on_demand_without_control_plane_restart() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / re-issue for an existing SpiffeId yields a \
         fresh leaf (distinct serial, new validity) with no control-plane restart)"
    );
}
