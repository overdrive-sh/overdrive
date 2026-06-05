//! `SystemdCredsKeyring` — the production [`Kek`] provider (built-in-ca /
//! GH #28, ADR-0063 D3/D6).
//!
//! Resolves a [`KekId`] to raw 256-bit KEK material from one of two sources,
//! in priority order:
//!
//! 1. **systemd-creds credential delivery** — when the control plane is
//!    started by systemd with `LoadCredentialEncrypted=<id>:<path>`, the
//!    decrypted credential is delivered as a file under
//!    `$CREDENTIALS_DIRECTORY/<id>` (systemd decrypts it at unit start; the
//!    key material never lands on disk in plaintext outside that tmpfs).
//!    This module reads that file — pure file I/O, no FFI, no `unsafe` (the
//!    crate is `#![forbid(unsafe_code)]`). The decrypted credential bytes ARE
//!    the KEK.
//! 2. **dev-only `OVERDRIVE_CA_KEK` fallback** — a passphrase supplied through
//!    the environment, accepted ONLY when systemd-creds delivered nothing.
//!    Gated and logged: in production (no explicit opt-in) the fallback is
//!    REFUSED — resolving falls through to [`KekError::NotFound`] rather than
//!    fabricating a KEK from a dev passphrase. The opt-in is the presence of
//!    `OVERDRIVE_CA_KEK_DEV_OPT_IN`. Per ADR-0063's Earned-Trust posture, an
//!    at-rest scheme keyed off an un-opted-in dev passphrase would be
//!    security theatre, so the gate is fail-closed.
//!
//! If neither source supplies material, [`resolve`](SystemdCredsKeyring::resolve)
//! returns [`KekError::NotFound`] — NEVER a zero/default KEK (which would make
//! the at-rest encryption meaningless; ADR-0063 D3 / Earned Trust). The boot
//! path treats `NotFound` as a refuse-to-start condition.
//!
//! # KEK material derivation
//!
//! The KEK is 256 bits ([`KEK_LEN`]). A delivered credential / dev passphrase
//! of arbitrary length is folded to exactly 256 bits via SHA-256 (a passphrase
//! is not itself uniformly-distributed key material; hashing it yields a
//! fixed-width key without imposing a "must be exactly 32 bytes" constraint on
//! the operator's credential). A credential that is already exactly
//! [`KEK_LEN`] raw bytes is used verbatim, so an operator who provisions raw
//! 256-bit key material gets it byte-for-byte.

use std::path::PathBuf;

use overdrive_core::ca::kek::{KEK_LEN, Kek, KekError, KekMaterial};
use overdrive_core::ca::root_key_envelope::KekId;
use ring::digest::{SHA256, digest};

/// Environment variable naming the systemd-creds credential directory.
/// systemd sets this for a unit that declares `LoadCredentialEncrypted=`.
const CREDENTIALS_DIRECTORY_ENV: &str = "CREDENTIALS_DIRECTORY";

/// Dev-only environment fallback carrying a KEK passphrase. Accepted ONLY when
/// the production opt-in ([`DEV_OPT_IN_ENV`]) is also set, OR when no
/// systemd-creds credential is present and the deployment has explicitly
/// opted in. Never used in a production posture without the opt-in.
const DEV_KEK_ENV: &str = "OVERDRIVE_CA_KEK";

/// Explicit opt-in for the dev `OVERDRIVE_CA_KEK` passphrase fallback. Its
/// presence (any value) acknowledges that the at-rest KEK is sourced from a
/// dev passphrase rather than a systemd-delivered credential — fail-closed by
/// default per ADR-0063 Earned-Trust.
const DEV_OPT_IN_ENV: &str = "OVERDRIVE_CA_KEK_DEV_OPT_IN";

/// The production [`Kek`] provider over systemd-creds credential delivery with
/// a gated dev passphrase fallback.
///
/// Holds no secret state — the credential / passphrase is read per `resolve`
/// call and folded to KEK material on the spot, never retained.
pub struct SystemdCredsKeyring {
    /// Override for the systemd credentials directory. `None` reads
    /// `$CREDENTIALS_DIRECTORY` from the environment at resolve time (the
    /// production path); `Some(dir)` pins it (tests, or a non-systemd
    /// deployment that stages credentials itself).
    credentials_dir: Option<PathBuf>,
}

impl Default for SystemdCredsKeyring {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemdCredsKeyring {
    /// Construct a provider that reads `$CREDENTIALS_DIRECTORY` from the
    /// environment at resolve time (the production systemd path).
    #[must_use]
    pub const fn new() -> Self {
        Self { credentials_dir: None }
    }

    /// Construct a provider pinned to an explicit credentials directory.
    ///
    /// Used by tests (and any non-systemd deployment that stages decrypted
    /// credentials itself) to point at a known directory instead of relying on
    /// the `$CREDENTIALS_DIRECTORY` environment variable.
    #[must_use]
    pub fn with_credentials_dir(dir: impl Into<PathBuf>) -> Self {
        Self { credentials_dir: Some(dir.into()) }
    }

    /// Resolve the systemd-creds credential directory for this call: the
    /// pinned override if present, else `$CREDENTIALS_DIRECTORY`.
    fn credentials_dir(&self) -> Option<PathBuf> {
        self.credentials_dir
            .clone()
            .or_else(|| std::env::var_os(CREDENTIALS_DIRECTORY_ENV).map(PathBuf::from))
    }

    /// Read the systemd-delivered credential file for `kek_id`, if present.
    ///
    /// Returns:
    /// * `Ok(Some(bytes))` — the credential file exists and was read.
    /// * `Ok(None)` — no credentials directory, or the file is absent.
    /// * `Err(reason)` — the directory/file exists but the read failed for a
    ///   reason other than absence (permissions, EIO). Surfaced as a backend
    ///   failure rather than collapsed into "not found".
    fn read_systemd_credential(&self, kek_id: &KekId) -> Result<Option<Vec<u8>>, String> {
        let Some(dir) = self.credentials_dir() else {
            return Ok(None);
        };
        let path = dir.join(kek_id.as_str());
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("read systemd credential {}: {err}", path.display())),
        }
    }

    /// Read the gated dev `OVERDRIVE_CA_KEK` passphrase, if the deployment has
    /// opted in.
    ///
    /// Returns `Some(passphrase_bytes)` ONLY when both `OVERDRIVE_CA_KEK` and
    /// `OVERDRIVE_CA_KEK_DEV_OPT_IN` are set. A set `OVERDRIVE_CA_KEK` WITHOUT
    /// the opt-in is refused (returns `None`) and a warning is logged once per
    /// resolve — a production posture must not key the root at rest off an
    /// un-opted-in dev passphrase.
    fn read_dev_fallback(kek_id: &KekId) -> Option<Vec<u8>> {
        let passphrase = std::env::var_os(DEV_KEK_ENV)?;
        if std::env::var_os(DEV_OPT_IN_ENV).is_none() {
            tracing::warn!(
                name: "ca.kek.dev_fallback_refused",
                kek_id = %kek_id,
                "OVERDRIVE_CA_KEK is set but OVERDRIVE_CA_KEK_DEV_OPT_IN is not; refusing the dev \
                 passphrase fallback (set OVERDRIVE_CA_KEK_DEV_OPT_IN to use it in development)",
            );
            return None;
        }
        tracing::warn!(
            name: "ca.kek.dev_fallback_used",
            kek_id = %kek_id,
            "resolving KEK from the dev OVERDRIVE_CA_KEK passphrase fallback (opted in via \
             OVERDRIVE_CA_KEK_DEV_OPT_IN); NOT for production use",
        );
        Some(passphrase.into_encoded_bytes())
    }
}

/// Fold arbitrary credential / passphrase bytes to exactly [`KEK_LEN`] (256
/// bits) of KEK material.
///
/// Raw input already exactly [`KEK_LEN`] bytes is used verbatim (an operator
/// who provisions raw 256-bit key material gets it byte-for-byte); any other
/// length is hashed with SHA-256 to a fixed-width key (a passphrase is not
/// uniformly-distributed key material, so it is hashed rather than truncated).
fn fold_to_kek_material(raw: &[u8]) -> KekMaterial {
    if let Ok(exact) = <[u8; KEK_LEN]>::try_from(raw) {
        return KekMaterial::new(exact);
    }
    let hashed = digest(&SHA256, raw);
    let mut bytes = [0u8; KEK_LEN];
    bytes.copy_from_slice(hashed.as_ref());
    KekMaterial::new(bytes)
}

impl Kek for SystemdCredsKeyring {
    fn resolve(&self, kek_id: &KekId) -> Result<KekMaterial, KekError> {
        // 1. systemd-creds credential delivery (production path).
        match self.read_systemd_credential(kek_id) {
            Ok(Some(bytes)) => return Ok(fold_to_kek_material(&bytes)),
            Ok(None) => {}
            Err(reason) => return Err(KekError::backend(reason)),
        }

        // 2. gated dev passphrase fallback.
        if let Some(passphrase) = Self::read_dev_fallback(kek_id) {
            return Ok(fold_to_kek_material(&passphrase));
        }

        // 3. neither source supplied material — refuse, never fabricate a KEK.
        Err(KekError::not_found(kek_id.clone()))
    }
}
