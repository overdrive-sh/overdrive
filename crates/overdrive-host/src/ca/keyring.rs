//! `SystemdCredsKeyring` — the production [`Kek`] provider (built-in-ca /
//! GH #28, ADR-0063 D3/D6).
//!
//! Resolves a [`KekId`] to raw 256-bit KEK material **held in kernel space**
//! via the Linux kernel keyring, with systemd-creds as the per-boot delivery
//! source. Per ADR-0063 D3 the KEK is *held* in the kernel keyring
//! (`add_key`/`keyctl`, `user`-type key in the service's session keyring) —
//! **not in the process heap** — so the persistent holder across the process
//! lifetime is the kernel, not a struct field on this adapter.
//!
//! # Boot flow ([`resolve`](SystemdCredsKeyring::resolve))
//!
//! 1. **Search the kernel keyring** for the KEK under a stable
//!    [`KekId`]-derived description. On a hit, read the 256-bit key back via
//!    `keyctl` and return it — the kernel was the holder.
//! 2. **On a miss**, obtain the raw KEK from its *delivery* source (in priority
//!    order), fold it to 256 bits, and `add_key` it into the session keyring
//!    under the same description. Then read it BACK out of the keyring (a fresh
//!    `keyctl` search + read), so the value `resolve` returns is the one the
//!    kernel holds — never the transient delivery buffer.
//!
//! Delivery sources, in priority order:
//!
//! 1. **systemd-creds credential delivery** — when the control plane is
//!    started by systemd with `LoadCredentialEncrypted=<id>:<path>`, the
//!    decrypted credential is delivered as a file under
//!    `$CREDENTIALS_DIRECTORY/<id>` (systemd decrypts it at unit start; the
//!    key material never lands on disk in plaintext outside that tmpfs). The
//!    decrypted credential bytes ARE the KEK.
//! 2. **dev-only `OVERDRIVE_CA_KEK` fallback** — a passphrase supplied through
//!    the environment, accepted ONLY when systemd-creds delivered nothing AND
//!    the deployment has opted in via `OVERDRIVE_CA_KEK_DEV_OPT_IN`. Gated and
//!    logged: in production (no explicit opt-in) the fallback is REFUSED. Per
//!    ADR-0063's Earned-Trust posture, an at-rest scheme keyed off an
//!    un-opted-in dev passphrase would be security theatre, so the gate is
//!    fail-closed.
//!
//! If neither the keyring nor any delivery source supplies material,
//! [`resolve`](SystemdCredsKeyring::resolve) returns [`KekError::NotFound`] —
//! NEVER a zero/default KEK (which would make the at-rest encryption
//! meaningless; ADR-0063 D3 / Earned Trust). The boot path treats `NotFound`
//! as a refuse-to-start condition. A kernel-keyring syscall failure
//! (permissions, quota, EIO) surfaces as [`KekError::Backend`] — distinct from
//! absence, so the operator gets the right remediation.
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
//!
//! # `#![forbid(unsafe_code)]`
//!
//! This module is unsafe-free. The kernel-keyring syscalls go through
//! `linux-keyutils`, a pure-Rust crate whose `unsafe` is internal — the
//! consumer surface is entirely safe, so the crate-level
//! `#![forbid(unsafe_code)]` (see `lib.rs`) is preserved.
//!
//! Note on transient buffers: the raw delivery buffer (the credential / dev
//! passphrase bytes read off the file / env) and the per-call `keyctl` read
//! buffer are unavoidable transient heap copies — `add_key` and `keyctl read`
//! are byte-slice syscalls. ADR-0063 D3's requirement is that the *persistent
//! holder across the process lifetime* be kernel space, which it is; the
//! transient feed/read buffers are fine. (No `zeroize` is in the dependency
//! graph and ADR-0063 D3 does not require zeroizing the transient feed buffer;
//! a dep is not added solely for this.)

use std::path::PathBuf;

use linux_keyutils::{KeyError, KeyRing, KeyRingIdentifier};
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

/// Default kernel-keyring description prefix for held KEKs. The full
/// description is `<prefix>:<kek_id>` (a `user`-type key in the session
/// keyring). Production uses this constant; tests pin a unique prefix per test
/// so parallel runs do not collide on the process/session-scoped keyring.
const DEFAULT_KEYRING_DESCRIPTION_PREFIX: &str = "overdrive:ca:kek";

/// The production [`Kek`] provider: holds the KEK in the Linux kernel session
/// keyring, with systemd-creds (+ a gated dev passphrase fallback) as the
/// per-boot delivery source.
///
/// Holds no secret state on the heap — the KEK lives in the kernel keyring;
/// the delivery credential is read per `resolve` call only on a keyring miss
/// and is never retained as a field.
pub struct SystemdCredsKeyring {
    /// Override for the systemd credentials directory. `None` reads
    /// `$CREDENTIALS_DIRECTORY` from the environment at resolve time (the
    /// production path); `Some(dir)` pins it (tests, or a non-systemd
    /// deployment that stages credentials itself).
    credentials_dir: Option<PathBuf>,

    /// Kernel-keyring description prefix.
    ///
    /// * `Some(prefix)` — explicit; the held key's description is
    ///   `<prefix>:<kek_id>`. Set via [`with_keyring_prefix`](Self::with_keyring_prefix)
    ///   (tests pin a unique-per-test prefix so the process/session-scoped
    ///   keyring entries do not collide).
    /// * `None` — derive the prefix. Production (no pinned credentials dir,
    ///   reads `$CREDENTIALS_DIRECTORY`) uses the stable
    ///   [`DEFAULT_KEYRING_DESCRIPTION_PREFIX`]. When a credentials dir is
    ///   pinned via [`with_credentials_dir`](Self::with_credentials_dir), the
    ///   pinned dir path is folded into the prefix so a provider scoped to a
    ///   distinct delivery directory holds its KEK under a distinct keyring
    ///   description — this is what lets the in-process boot tests model
    ///   distinct "reboots" (distinct delivery sources) without one boot's
    ///   held KEK leaking into another's lookup, since the kernel session
    ///   keyring outlives a single test's process.
    keyring_prefix: Option<String>,
}

impl Default for SystemdCredsKeyring {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemdCredsKeyring {
    /// Construct a provider that reads `$CREDENTIALS_DIRECTORY` from the
    /// environment at resolve time (the production systemd path) and holds the
    /// KEK in the session keyring under the default description prefix.
    #[must_use]
    pub const fn new() -> Self {
        Self { credentials_dir: None, keyring_prefix: None }
    }

    /// Construct a provider pinned to an explicit credentials directory.
    ///
    /// Used by tests (and any non-systemd deployment that stages decrypted
    /// credentials itself) to point at a known directory instead of relying on
    /// the `$CREDENTIALS_DIRECTORY` environment variable.
    #[must_use]
    pub fn with_credentials_dir(dir: impl Into<PathBuf>) -> Self {
        Self { credentials_dir: Some(dir.into()), keyring_prefix: None }
    }

    /// Override the kernel-keyring description prefix (chainable).
    ///
    /// The held key's description becomes `<prefix>:<kek_id>`. Tests use a
    /// unique-per-test prefix so the process/session-scoped keyring entries do
    /// not collide across parallel tests, and so each test can locate and
    /// remove its own entry on cleanup. An explicit prefix takes precedence
    /// over the credentials-dir-derived default.
    #[must_use]
    pub fn with_keyring_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.keyring_prefix = Some(prefix.into());
        self
    }

    /// The kernel-keyring description this provider holds `kek_id` under.
    ///
    /// The prefix is the explicit [`with_keyring_prefix`](Self::with_keyring_prefix)
    /// value if set; otherwise [`DEFAULT_KEYRING_DESCRIPTION_PREFIX`] with the
    /// pinned credentials-dir path folded in (when a dir is pinned), so a
    /// provider scoped to a distinct delivery directory holds its KEK under a
    /// distinct description. Production (no pinned dir) uses the stable
    /// default prefix verbatim.
    #[must_use]
    pub fn keyring_description(&self, kek_id: &KekId) -> String {
        format!("{}:{}", self.keyring_prefix_resolved(), kek_id.as_str())
    }

    /// Resolve the keyring description prefix per the [`keyring_prefix`] field
    /// contract.
    fn keyring_prefix_resolved(&self) -> String {
        if let Some(explicit) = &self.keyring_prefix {
            return explicit.clone();
        }
        self.credentials_dir.as_ref().map_or_else(
            // Production path (reads `$CREDENTIALS_DIRECTORY`) — stable prefix.
            || DEFAULT_KEYRING_DESCRIPTION_PREFIX.to_owned(),
            // Pinned credentials dir (tests / non-systemd staging) — fold the
            // dir path into the prefix so distinct delivery sources hold under
            // distinct keyring descriptions (a session keyring outlives a test
            // process). SHA-256 keeps the description charset valid + bounded.
            |dir| {
                let hashed = digest(&SHA256, dir.as_os_str().as_encoded_bytes());
                format!("{DEFAULT_KEYRING_DESCRIPTION_PREFIX}:dir-{}", hex_lower(hashed.as_ref()))
            },
        )
    }

    /// Resolve the session keyring (creating it if absent), mapping any
    /// keyring-backend syscall failure to [`KekError::Backend`].
    fn session_keyring() -> Result<KeyRing, KekError> {
        KeyRing::from_special_id(KeyRingIdentifier::Session, true)
            .map_err(|err| KekError::backend(format!("open session keyring: {err:?}")))
    }

    /// Read the KEK back out of the kernel keyring under `description`.
    ///
    /// Returns:
    /// * `Ok(Some(material))` — the key is present and read back as exactly
    ///   [`KEK_LEN`] bytes (the kernel was the holder).
    /// * `Ok(None)` — no key with that description exists in the keyring tree.
    /// * `Err(Backend)` — a keyring syscall failed for a reason other than
    ///   absence, or the held key is not [`KEK_LEN`] bytes (corruption).
    fn read_from_keyring(
        keyring: KeyRing,
        description: &str,
    ) -> Result<Option<KekMaterial>, KekError> {
        let key = match keyring.search(description) {
            Ok(key) => key,
            Err(KeyError::KeyDoesNotExist) => return Ok(None),
            Err(err) => {
                return Err(KekError::backend(format!(
                    "search kernel keyring for `{description}`: {err:?}"
                )));
            }
        };
        let bytes = key.read_to_vec().map_err(|err| {
            KekError::backend(format!("read KEK `{description}` from kernel keyring: {err:?}"))
        })?;
        let exact = <[u8; KEK_LEN]>::try_from(bytes.as_slice()).map_err(|_| {
            KekError::backend(format!(
                "kernel keyring KEK `{description}` is {} bytes, expected {KEK_LEN}",
                bytes.len()
            ))
        })?;
        Ok(Some(KekMaterial::new(exact)))
    }

    /// `add_key` the folded KEK material into the session keyring under
    /// `description` (a `user`-type key). Maps any syscall failure to
    /// [`KekError::Backend`].
    fn add_to_keyring(
        keyring: KeyRing,
        description: &str,
        material: &KekMaterial,
    ) -> Result<(), KekError> {
        keyring.add_key(description, material.expose_secret()).map_err(|err| {
            KekError::backend(format!("add KEK `{description}` to kernel keyring: {err:?}"))
        })?;
        Ok(())
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

    /// Obtain the raw KEK from its delivery source (systemd-creds, then the
    /// gated dev fallback), folded to [`KEK_LEN`] material.
    ///
    /// Returns `Ok(None)` when no delivery source supplies material (the
    /// refuse-to-start case); `Err(Backend)` when a delivery read itself fails.
    fn deliver_material(&self, kek_id: &KekId) -> Result<Option<KekMaterial>, KekError> {
        // 1. systemd-creds credential delivery (production path).
        match self.read_systemd_credential(kek_id) {
            Ok(Some(bytes)) => return Ok(Some(fold_to_kek_material(&bytes))),
            Ok(None) => {}
            Err(reason) => return Err(KekError::backend(reason)),
        }
        // 2. gated dev passphrase fallback.
        if let Some(passphrase) = Self::read_dev_fallback(kek_id) {
            return Ok(Some(fold_to_kek_material(&passphrase)));
        }
        // 3. neither delivery source supplied material.
        Ok(None)
    }
}

/// Fold arbitrary credential / passphrase bytes to exactly [`KEK_LEN`] (256
/// bits) of KEK material.
///
/// Raw input already exactly [`KEK_LEN`] bytes is used verbatim (an operator
/// who provisions raw 256-bit key material gets it byte-for-byte); any other
/// length is hashed with SHA-256 to a fixed-width key (a passphrase is not
/// uniformly-distributed key material, so it is hashed rather than truncated).
/// Lowercase-hex-encode `bytes` (for a keyring-description-safe digest).
fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

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
        let description = self.keyring_description(kek_id);
        let keyring = Self::session_keyring()?;

        // 1. The kernel keyring is the holder: if the KEK is already held,
        //    read it back via keyctl and return it — no heap-resident copy
        //    persists across resolve calls (ADR-0063 D3).
        if let Some(material) = Self::read_from_keyring(keyring, &description)? {
            return Ok(material);
        }

        // 2. Keyring miss — obtain the KEK from its delivery source and
        //    install it into the kernel keyring.
        let Some(delivered) = self.deliver_material(kek_id)? else {
            // Neither the keyring nor any delivery source supplied material —
            // refuse, never fabricate a KEK (ADR-0063 D3 / Earned Trust).
            return Err(KekError::not_found(kek_id.clone()));
        };
        Self::add_to_keyring(keyring, &description, &delivered)?;

        // 3. Read the KEK BACK out of the keyring so the value returned is the
        //    one the kernel now holds (the round-trip IS the D8 boot probe);
        //    a key that failed to install would surface here rather than
        //    handing back the transient delivery buffer.
        Self::read_from_keyring(keyring, &description)?.ok_or_else(|| {
            KekError::backend(format!(
                "KEK `{description}` vanished from kernel keyring immediately after add_key"
            ))
        })
    }
}
