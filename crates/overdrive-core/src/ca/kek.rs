//! `Kek` provider port — resolves a [`KekId`] to raw 256-bit
//! key-encryption-key material (built-in-ca / GH #28, ADR-0063 D3/D6).
//!
//! The KEK protects the root CA private key at rest (ADR-0063 D2/D4): the
//! host AEAD codec HKDF-derives a per-use subkey from the KEK and
//! AES-256-GCM-seals the root key under it. This module defines the *pure*
//! provider port — `overdrive-core` (class `core`) holds the trait and the
//! opaque KEK-material newtype, but NEVER a keyring/FFI backend. The real
//! provider (`SystemdCredsKeyring` over the Linux kernel keyring) is a
//! host-adapter concern in `overdrive-host` and lands in a later slice; this
//! port keeps the codec's KEK source swappable and dst-lint-clean.
//!
//! # Why the KEK material is opaque
//!
//! [`KekMaterial`] wraps `[u8; 32]` (256-bit) and is **deliberately
//! incomplete** as a newtype: it has NO `Display`, NO `Serialize`, NO
//! `FromStr`. The KEK is the trust anchor's anchor — rendering or serialising
//! it would risk leaking the secret into logs, snapshots, or audit rows. The
//! only observable accessor is [`KekMaterial::expose_secret`], named to make
//! every read site grep-visible. `KekId` (the *identifier*, not the secret)
//! carries the full `FromStr`/`Display`/serde surface and lives in
//! [`crate::ca::root_key_envelope`].

use crate::ca::root_key_envelope::KekId;

/// Byte length of a KEK — 256 bits, matching the AES-256-GCM subkey the host
/// codec derives from it (ADR-0063 D4).
pub const KEK_LEN: usize = 32;

/// Raw 256-bit key-encryption-key material resolved by a [`Kek`] provider.
///
/// An opaque secret-bytes newtype: no `Display`, no `Serialize`, no `FromStr`
/// — the KEK must never leak into a log line, a serialised record, or an audit
/// row. The single observable accessor is [`expose_secret`](Self::expose_secret),
/// whose name makes each read site auditable.
#[derive(Clone, PartialEq, Eq)]
pub struct KekMaterial([u8; KEK_LEN]);

impl KekMaterial {
    /// Wrap raw 256-bit KEK bytes.
    #[must_use]
    pub const fn new(bytes: [u8; KEK_LEN]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw KEK bytes.
    ///
    /// Named `expose_secret` (not `as_bytes`) so every read of the raw key
    /// material is grep-visible at the call site — this is the one place the
    /// secret leaves the newtype, and it should never reach a log/sink.
    #[must_use]
    pub const fn expose_secret(&self) -> &[u8; KEK_LEN] {
        &self.0
    }
}

/// A redacting `Debug` — never prints the secret bytes.
///
/// `KekMaterial` derives no `Debug`; this hand-written impl prints a fixed
/// placeholder so an accidental `{:?}` on a struct embedding a KEK cannot leak
/// the key into a log line.
impl std::fmt::Debug for KekMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KekMaterial(<redacted 256-bit>)")
    }
}

/// A KEK-resolution failure.
///
/// Distinct per failure mode (`.claude/rules/development.md` § "Distinct
/// failure modes get distinct error variants"). A provider that cannot find
/// the requested KEK surfaces [`NotFound`](KekError::NotFound) — never a
/// silent zero/default KEK (which would make the at-rest encryption
/// meaningless, ADR-0063 Earned Trust).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KekError {
    /// No KEK is registered for the requested identity. The provider has no
    /// material to resolve — the boot path refuses to start rather than
    /// fabricate a KEK (ADR-0063 D3 / Earned Trust).
    #[error("no KEK registered for id `{kek_id}`")]
    NotFound {
        /// The KEK identity that could not be resolved.
        kek_id: KekId,
    },

    /// The provider's backend failed (keyring read error, credential delivery
    /// failure). Carries a human-readable reason; host adapters map their
    /// backend-specific errors into this variant.
    #[error("KEK provider backend failed: {reason}")]
    Backend {
        /// Human-readable explanation of the backend failure.
        reason: String,
    },
}

impl KekError {
    /// Construct a [`NotFound`](KekError::NotFound) for an unresolvable id.
    #[must_use]
    pub const fn not_found(kek_id: KekId) -> Self {
        Self::NotFound { kek_id }
    }

    /// Construct a [`Backend`](KekError::Backend) failure.
    #[must_use]
    pub fn backend(reason: impl Into<String>) -> Self {
        Self::Backend { reason: reason.into() }
    }
}

/// The KEK provider port.
///
/// A pure trait — no impl, no keyring, no FFI (ADR-0063 D3/D6). The host
/// adapter wires it to the Linux kernel keyring (`SystemdCredsKeyring`, a
/// later slice); tests wire an in-memory fixture. The host AEAD codec
/// ([`overdrive-host`] `RootKeyAeadCodec`) takes `&dyn Kek` and resolves the
/// KEK material at seal/open time — the KEK never enters `overdrive-core`,
/// only its identity does.
pub trait Kek: Send + Sync {
    /// Resolve `kek_id` to its raw 256-bit KEK material.
    ///
    /// # Preconditions
    /// `kek_id` names a KEK the provider can supply (delivered at boot via
    /// systemd-creds in production; pre-registered in a fixture under test).
    ///
    /// # Postconditions
    /// On `Ok`, the returned [`KekMaterial`] is the exact 256-bit key bound to
    /// `kek_id` — resolving the same id twice yields equal material.
    ///
    /// # Errors
    /// * [`KekError::NotFound`] when no KEK is registered for `kek_id` — the
    ///   provider never substitutes a zero/default KEK.
    /// * [`KekError::Backend`] when the provider's backend (keyring, credential
    ///   store) itself fails.
    fn resolve(&self, kek_id: &KekId) -> Result<KekMaterial, KekError>;
}
