//! Integration — `SystemdCredsKeyring` holds the root-key KEK in the **real
//! Linux kernel keyring** (`add_key`/`keyctl`), per ADR-0063 D3 / D8
//! (built-in-ca / GH #28).
//!
//! Layer 3 (gated `integration-tests`, runs via Lima as root — exercises the
//! real session keyring through `linux-keyutils`). These prove the ADR-0063 D3
//! holder invariant: the persistent holder of the KEK across the process
//! lifetime is **kernel space**, not the process heap. The proof is
//! independent of the adapter — a second, separately-constructed `KeyRing`
//! search finds the `user`-type key the adapter installed, and a re-resolve
//! after the delivery source is gone still returns the KEK (it came from the
//! kernel, not a re-read of the credential file).
//!
//! Scenarios trace to: US-CA-02 (KEK delivery + holder), ADR-0063 D3 ("held
//! in kernel space via the Linux kernel keyring … not in the process heap"),
//! D8 Earned-Trust probe ("KEK present in keyring — probe `keyctl`/`add_key`
//! round-trips the KEK"), and the `KekError::NotFound` refuse-to-start floor.
//! Tags: `@real-io` `@adapter-integration` `@S-02`.
//!
//! Keyring scoping & cleanup: the session keyring is process/session-scoped, so
//! every test pins a UNIQUE-per-test description prefix (via
//! `with_keyring_prefix`) so parallel tests never collide, and installs a
//! `KeyringCleanup` RAII guard that invalidates the entry on drop so a leaked
//! keyring key cannot poison a later run.

use linux_keyutils::{KeyError, KeyRing, KeyRingIdentifier};
use overdrive_core::ca::kek::{KEK_LEN, Kek, KekError};
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_host::ca::SystemdCredsKeyring;
use serial_test::serial;
use tempfile::TempDir;

/// The boot KEK id the suite resolves under.
fn kek_id() -> KekId {
    KekId::new("overdrive-ca-root-v1").expect("KekId parses")
}

/// A description prefix unique to a single test, so the process/session-scoped
/// keyring entries never collide across parallel tests. `tag` distinguishes
/// callers; the PID + a monotonic counter make it unique even within one tag.
fn unique_prefix(tag: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("overdrive-test:{tag}:{}:{n}", std::process::id())
}

/// Stage a 32-byte systemd-creds credential file named for `kek_id` under
/// `dir`, so `SystemdCredsKeyring::with_credentials_dir(dir)` resolves the KEK
/// with no environment dependency.
fn stage_credential(dir: &TempDir, kek_id: &KekId, byte: u8) {
    std::fs::write(dir.path().join(kek_id.as_str()), [byte; KEK_LEN])
        .expect("write systemd-creds KEK credential");
}

/// RAII cleanup: invalidate the kernel-keyring entry the test installed, so a
/// leaked key cannot poison later runs in the same session.
struct KeyringCleanup {
    description: String,
}

impl Drop for KeyringCleanup {
    fn drop(&mut self) {
        if let Ok(keyring) = KeyRing::from_special_id(KeyRingIdentifier::Session, false)
            && let Ok(key) = keyring.search(&self.description)
        {
            // Best-effort — the test already asserted what it needed; a failed
            // invalidate here only risks a stale entry the unique per-test
            // description already isolates.
            let _ = key.invalidate();
        }
    }
}

/// Independently (NOT through the adapter) search the real session keyring for
/// `description` and read the held bytes back. Proves the adapter installed a
/// `user`-type key the kernel actually holds.
fn independent_keyring_read(description: &str) -> Option<Vec<u8>> {
    let keyring = KeyRing::from_special_id(KeyRingIdentifier::Session, false).ok()?;
    let key = match keyring.search(description) {
        Ok(key) => key,
        Err(KeyError::KeyDoesNotExist) => return None,
        Err(err) => panic!("independent keyring search failed: {err:?}"),
    };
    Some(key.read_to_vec().expect("independent keyctl read"))
}

// ---------------------------------------------------------------------------
// ADR-0063 D3 — the kernel keyring is the holder (round-trip + independence)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — ADR-0063 D3/D8: resolving a KEK
/// installs it into the real kernel keyring and reads it back byte-identically.
/// An INDEPENDENT keyring search (a separate `KeyRing` handle, not the adapter)
/// finds the `user`-type key the adapter added — so the KEK is genuinely held
/// in kernel space, not merely returned from a heap copy. The keyctl round-trip
/// IS the D8 "KEK present in keyring" boot probe.
#[test]
fn resolved_kek_is_held_in_kernel_keyring_and_read_back_identically() {
    // GIVEN a staged systemd-creds credential and an adapter pinned to a
    // unique-per-test keyring description.
    let creds = TempDir::new().expect("creds tempdir");
    let id = kek_id();
    stage_credential(&creds, &id, 0x42);
    let keyring = SystemdCredsKeyring::with_credentials_dir(creds.path())
        .with_keyring_prefix(unique_prefix("held"));
    let description = keyring.keyring_description(&id);
    let _cleanup = KeyringCleanup { description: description.clone() };

    // Precondition: the kernel keyring does NOT already hold this key (a unique
    // per-test description guarantees it) — so a found key later is the
    // adapter's doing, not a pre-existing entry.
    assert!(
        independent_keyring_read(&description).is_none(),
        "unique per-test description must not pre-exist in the keyring"
    );

    // WHEN the KEK is resolved.
    let resolved = keyring.resolve(&id).expect("first resolve installs + reads back the KEK");

    // THEN an INDEPENDENT keyring search finds the held key, byte-identical to
    // what `resolve` returned — proving the kernel keyring is the holder.
    let held = independent_keyring_read(&description)
        .expect("the adapter must have installed a user-type key the kernel holds");
    assert_eq!(
        held.as_slice(),
        resolved.expose_secret().as_slice(),
        "the kernel-held KEK bytes must equal the bytes resolve() returned"
    );
    assert_eq!(held.len(), KEK_LEN, "the held key must be exactly the 256-bit KEK");

    // AND a second resolve returns byte-identical material (idempotent — the
    // kernel keyring is now the source).
    let again = keyring.resolve(&id).expect("second resolve reads the held KEK");
    assert_eq!(
        again.expose_secret().as_slice(),
        resolved.expose_secret().as_slice(),
        "resolving the same id twice yields equal material (Kek port postcondition)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` — ADR-0063 D3: the kernel keyring,
/// not the credential file, is the persistent holder. After the FIRST resolve
/// installs the KEK, the systemd-creds delivery directory is removed entirely;
/// a subsequent resolve STILL returns the same KEK because it is read back out
/// of the kernel keyring, never re-read from the now-absent delivery source.
#[test]
fn resolve_reads_kek_from_kernel_keyring_after_delivery_source_removed() {
    // GIVEN a staged credential the first resolve consumes.
    let creds = TempDir::new().expect("creds tempdir");
    let id = kek_id();
    stage_credential(&creds, &id, 0x7e);
    let keyring = SystemdCredsKeyring::with_credentials_dir(creds.path())
        .with_keyring_prefix(unique_prefix("survives-delivery"));
    let description = keyring.keyring_description(&id);
    let _cleanup = KeyringCleanup { description };

    let first = keyring.resolve(&id).expect("first resolve installs the KEK from delivery");

    // WHEN the delivery source disappears (credential file deleted), so any
    // re-read of the credential would now fail to find material.
    std::fs::remove_file(creds.path().join(id.as_str())).expect("remove staged credential");

    // THEN a subsequent resolve STILL returns the same KEK — it came from the
    // kernel keyring, proving the kernel (not the credential file) is the
    // holder across the process lifetime.
    let second =
        keyring.resolve(&id).expect("resolve after delivery removed must read the held KEK");
    assert_eq!(
        second.expose_secret().as_slice(),
        first.expose_secret().as_slice(),
        "KEK must be read from the kernel keyring, not re-read from the absent credential file"
    );
}

// ---------------------------------------------------------------------------
// Earned-Trust floor — absent KEK refuses (NotFound), never a default KEK
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — ADR-0063 D3 / D8: an
/// EMPTY keyring (unique description never installed) + no systemd-creds
/// delivery + no dev `OVERDRIVE_CA_KEK` opt-in resolves to
/// `KekError::NotFound`, never a zero/default KEK (which would make the at-rest
/// encryption meaningless). The env-mutating dev-fallback vars are cleared
/// under `#[serial(env)]`.
#[test]
#[serial(env)]
fn absent_kek_with_no_delivery_and_no_dev_opt_in_is_not_found() {
    // GIVEN an EMPTY credentials directory (no credential staged), a unique
    // keyring description that does not pre-exist, and no dev opt-in.
    let empty_creds = TempDir::new().expect("empty-creds tempdir");
    let id = kek_id();
    let keyring = SystemdCredsKeyring::with_credentials_dir(empty_creds.path())
        .with_keyring_prefix(unique_prefix("absent"));
    let description = keyring.keyring_description(&id);
    let _cleanup = KeyringCleanup { description: description.clone() };

    // SAFETY: `#[serial(env)]` guarantees exclusive access to the process
    // environment for the duration of this test, so removing the dev-fallback
    // vars cannot race another test.
    unsafe {
        std::env::remove_var("OVERDRIVE_CA_KEK");
        std::env::remove_var("OVERDRIVE_CA_KEK_DEV_OPT_IN");
    }

    // WHEN the KEK is resolved with no keyring entry and no delivery source.
    let result = keyring.resolve(&id);

    // THEN it refuses with NotFound — never a fabricated default KEK.
    assert!(
        matches!(result, Err(KekError::NotFound { .. })),
        "absent KEK (empty keyring, no delivery, no dev opt-in) must be NotFound, got {result:?}"
    );

    // AND nothing was installed into the kernel keyring (the refuse path must
    // not leave a partial/default entry behind).
    assert!(
        independent_keyring_read(&description).is_none(),
        "the NotFound refuse path must not install any keyring entry"
    );
}
