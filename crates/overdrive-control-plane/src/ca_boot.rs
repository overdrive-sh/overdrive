//! CA boot composition root — generate-or-load the persistent root CA with an
//! Earned-Trust probe (built-in-ca / GH #28, ADR-0063 D2/D3/D8).
//!
//! This is the focused boot seam that supersedes ADR-0010's *ephemeral* CA: on
//! first boot the control plane generates the root, envelope-encrypts its
//! private key under the operator KEK, and persists the sealed record (plus the
//! public root cert material) to the [`IntentStore`]. On every subsequent boot
//! it loads the persisted record, decrypts it under the KEK, and reuses the
//! **same** root identity (same public key / same cert) — so every workload
//! identity issued under the old root keeps verifying.
//!
//! # Earned Trust — wire → probe → use (ADR-0063 D3/D8)
//!
//! [`boot_ca`] PROBES before it accepts the root for use:
//!
//! * **KEK present** — the KEK must resolve through the provider. An absent KEK
//!   (no systemd-creds credential, no opted-in dev fallback) refuses startup
//!   BEFORE any issuance and mints NO throwaway KEK ([`CaBootError::KekUnavailable`]).
//! * **Envelope decrypts** — a persisted record must AES-GCM-open under the
//!   resolved KEK. A tampered / wrong-KEK / corrupt record refuses startup with
//!   a typed error and emits `health.startup.refused`, and mints NO new root
//!   ([`CaBootError::EnvelopeDecrypt`]) — a silent re-mint would orphan every
//!   issued identity.
//!
//! All failure paths are fail-closed: the control plane refuses to start rather
//! than degrade to an un-protected or re-minted root.

use std::sync::Arc;

use overdrive_core::ca::kek::{Kek, KekError};
use overdrive_core::ca::root_key_envelope::{KekId, RootCaKeyRecord};
use overdrive_core::traits::ca::{
    Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, IntermediateHandle, RootCaHandle,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::{CertSerial, NodeId};
use overdrive_host::ca::RootKeyAeadCodec;

/// IntentStore key under which the sealed root-key envelope bytes live.
const ROOT_KEY_ENVELOPE_KEY: &[u8] = b"ca/root/key-envelope/v1";

/// IntentStore key under which the public root-cert material lives (PEM + DER
/// + serial, newline-framed). Public material — no secret here; persisting it
/// lets a subsequent boot present the byte-identical root identity without
/// re-self-signing (which would change the cert even at the same key).
const ROOT_CERT_MATERIAL_KEY: &[u8] = b"ca/root/cert-material/v1";

/// IntentStore key under which the single node-intermediate public cert
/// material lives (PEM + DER + serial, newline-framed). Single-node scope:
/// one node → one intermediate (multi-node per-node intermediates + node
/// attestation are tracked in #36, not built here).
const NODE_INTERMEDIATE_MATERIAL_KEY: &[u8] = b"ca/node/intermediate-material/v1";

/// IntentStore key under which the sealed node-intermediate key envelope bytes
/// live (built-in-ca / GH #28). The intermediate signing key is sealed under
/// the SAME operator KEK as the root, but with a DISTINCT HKDF `info` label
/// (`RootKeyAeadCodec::seal_intermediate`) so the two subkeys are
/// domain-separated. Persisting the sealed key is what makes the node
/// intermediate STABLE across restart — without it the intermediate is
/// ephemeral per process and every SVID signed under a prior boot's
/// intermediate fails to chain to the refreshed trust bundle.
const NODE_INTERMEDIATE_KEY_ENVELOPE_KEY: &[u8] = b"ca/node/intermediate-key-envelope/v1";

/// A CA boot failure — fail-closed startup refusal (ADR-0063 D3 Earned Trust).
#[derive(Debug, thiserror::Error)]
pub enum CaBootError {
    /// The KEK could not be resolved from the provider — no systemd-creds
    /// credential and no opted-in dev fallback. Startup is refused BEFORE any
    /// issuance; no throwaway KEK is minted (ADR-0063 D3 / Earned Trust). The
    /// boot path emits `health.startup.refused` before returning this.
    #[error("KEK `{kek_id}` is unavailable; refusing to start (source: {source})")]
    KekUnavailable {
        /// The KEK identity that could not be resolved.
        kek_id: KekId,
        /// The underlying provider failure.
        #[source]
        source: KekError,
    },

    /// A persisted root-key envelope failed to decrypt under the resolved KEK
    /// (tampered / corrupt / wrong KEK). Startup is refused with NO silent
    /// re-mint (a re-mint orphans every issued identity, ADR-0063 D3). The boot
    /// path emits `health.startup.refused` before returning this.
    #[error("persisted root-key envelope failed to decrypt; refusing to start (cause: {source})")]
    EnvelopeDecrypt {
        /// The underlying AEAD-open failure (tampered vs wrong-KEK distinct).
        #[source]
        source: CaError,
    },

    /// The CA adapter failed to mint or compose the root (first boot), or the
    /// persisted public cert material was malformed.
    #[error("root CA generation/load failed: {source}")]
    Ca {
        /// The underlying CA failure.
        #[source]
        source: CaError,
    },

    /// The root-key envelope failed to SERIALIZE before persistence (the
    /// write-side counterpart to [`EnvelopeDecrypt`](Self::EnvelopeDecrypt)).
    /// The structured cause is rkyv-archival shape, never key plaintext.
    #[error("root-key envelope serialization failed: {source}")]
    Envelope {
        /// The underlying envelope-archive failure.
        #[from]
        source: overdrive_core::codec::EnvelopeError,
    },

    /// The IntentStore read/write failed while persisting or loading the root.
    #[error(transparent)]
    Intent(#[from] overdrive_core::traits::intent_store::IntentStoreError),
}

/// The KEK identity the built-in CA seals its root under.
///
/// A single-node deployment uses one stable KEK identity; the operator
/// provisions the matching KEK via systemd-creds (or the dev fallback).
#[must_use]
pub fn root_kek_id() -> KekId {
    // Constant, validated label — `unreachable!` documents the invariant
    // (a hardcoded valid label can never fail `KekId::new`).
    KekId::new("overdrive-ca-root")
        .unwrap_or_else(|_| unreachable!("`overdrive-ca-root` is a valid KekId label"))
}

/// Generate-or-load the persistent root CA, with the Earned-Trust probe.
///
/// `redb_path` is the on-disk path of the `IntentStore`'s redb file (supplied
/// by the caller that opened the store); it is threaded into the corrupt-/
/// undecodable-envelope path so the `health.startup.refused` event and the
/// `IntentStoreError::Envelope` remediation hint name the real file to inspect
/// or delete, rather than an unactionable placeholder.
///
/// # Behaviour
///
/// * **KEK probe** — resolve `kek_id` through `kek`. On failure, emit
///   `health.startup.refused` and return [`CaBootError::KekUnavailable`]
///   BEFORE generating or persisting anything (no throwaway KEK).
/// * **First boot** (no persisted envelope) — `ca.root()` generates the root,
///   the codec seals its private key under the KEK, and both the sealed
///   envelope and the public cert material are persisted. Returns the freshly
///   minted [`RootCaHandle`].
/// * **Subsequent boot** (persisted envelope present) — decrypt the envelope
///   under the KEK (probe), reconstruct the [`RootCaHandle`] from the persisted
///   public cert material + the decrypted key. On decrypt failure, emit
///   `health.startup.refused` and return [`CaBootError::EnvelopeDecrypt`] with
///   NO re-mint.
///
/// # Errors
///
/// See [`CaBootError`] — every variant is a fail-closed startup refusal.
pub async fn boot_ca(
    ca: &dyn Ca,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    intent: &Arc<dyn IntentStore>,
    redb_path: &std::path::Path,
) -> Result<RootCaHandle, CaBootError> {
    // Earned-Trust probe (a): the KEK MUST resolve before we generate, load,
    // or persist anything. Refuse-to-start on absence — never mint a throwaway
    // KEK that would make at-rest encryption meaningless.
    kek.resolve(kek_id).map_err(|source| {
        tracing::error!(
            name: "health.startup.refused",
            kek_id = %kek_id,
            cause = %source,
            "KEK unavailable at boot; control-plane refusing to start (no throwaway KEK minted)",
        );
        CaBootError::KekUnavailable { kek_id: kek_id.clone(), source }
    })?;

    match intent.get(ROOT_KEY_ENVELOPE_KEY).await? {
        Some(envelope_bytes) => {
            load_persistent_root(ca, kek, kek_id, codec, intent, redb_path, &envelope_bytes).await
        }
        None => generate_and_persist_root(ca, kek, kek_id, codec, intent).await,
    }
}

/// Bootstrap the single node intermediate CA (single-node: one node → one
/// intermediate), signed by the root that `boot_ca` composed — **stable across
/// restart** (built-in-ca / GH #28).
///
/// # Behaviour — generate-or-load (mirrors [`boot_ca`])
///
/// * **First boot** (no persisted intermediate-key envelope) — issue the node
///   intermediate through [`Ca::issue_intermediate`], seal its private signing
///   key under the KEK (distinct HKDF `info` label from the root —
///   [`RootKeyAeadCodec::seal_intermediate`]), and persist BOTH the sealed key
///   envelope AND the public cert material to the [`IntentStore`]. Returns the
///   freshly minted [`IntermediateHandle`].
/// * **Subsequent boot** (persisted intermediate-key envelope present) —
///   decrypt the envelope under the KEK (Earned-Trust probe), reconstruct the
///   [`IntermediateHandle`] from the persisted public cert material + the
///   decrypted key, and re-seed the CA adapter via
///   [`Ca::adopt_persisted_intermediate`] BEFORE any issuance — so issuance
///   signs under the SAME intermediate relying parties already anchor chains
///   on, rather than a freshly-minted ephemeral one (the chain-break this
///   closes). Returns the adopted handle.
///
/// The intermediate is what the node presents to issue workload SVIDs under —
/// the node does not run workloads it cannot identify.
///
/// # Fail-loud (ADR-0063, reconciler-discipline + Earned Trust)
///
/// * First boot, root key unavailable (decrypt failed upstream): the signing
///   failure surfaces from `issue_intermediate` as a typed [`CaError`]; this fn
///   emits `health.startup.refused` and returns [`CaBootError::Ca`] WITHOUT
///   persisting anything. No half-provisioned state is left behind — there is
///   no adopt-and-skip of a partial intermediate.
/// * Subsequent boot, envelope decrypt failure (tampered / wrong KEK / corrupt):
///   emit `health.startup.refused` and return [`CaBootError::EnvelopeDecrypt`]
///   with NO silent re-issue — a silent re-mint would orphan every SVID signed
///   under the persisted intermediate.
///
/// # Errors
///
/// [`CaBootError::Ca`] when the intermediate cannot be signed (root key
/// unavailable) or adoption diverges; [`CaBootError::EnvelopeDecrypt`] when the
/// persisted intermediate-key envelope cannot be opened under the KEK;
/// [`CaBootError::Intent`] when persistence/load fails.
///
/// Single-node only: multi-node per-node intermediates + node attestation are
/// tracked in #36 and are NOT built here.
pub async fn bootstrap_node_intermediate(
    ca: &dyn Ca,
    node: &NodeId,
    intent: &Arc<dyn IntentStore>,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    redb_path: &std::path::Path,
) -> Result<IntermediateHandle, CaBootError> {
    match intent.get(NODE_INTERMEDIATE_KEY_ENVELOPE_KEY).await? {
        Some(envelope_bytes) => {
            load_persistent_intermediate(
                ca,
                node,
                kek,
                kek_id,
                codec,
                intent,
                redb_path,
                &envelope_bytes,
            )
            .await
        }
        None => generate_and_persist_intermediate(ca, node, kek, kek_id, codec, intent).await,
    }
}

/// First boot: sign the node intermediate, seal its key under the KEK
/// (intermediate `info` label), and persist both the sealed key envelope and
/// the public cert material.
async fn generate_and_persist_intermediate(
    ca: &dyn Ca,
    node: &NodeId,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    intent: &Arc<dyn IntentStore>,
) -> Result<IntermediateHandle, CaBootError> {
    // Sign the node intermediate by the root. A root-key-unavailable (decrypt
    // failed upstream) signing failure surfaces here as a typed `CaError`. Fail
    // loudly: emit `health.startup.refused` and return BEFORE any persistence,
    // so no half-provisioned intermediate is left behind (reconciler-discipline
    // — no adopt-and-skip of a partial; the node does not run workloads it
    // cannot identify).
    let intermediate = ca.issue_intermediate(node).map_err(|source| {
        tracing::error!(
            name: "health.startup.refused",
            node = %node,
            cause = %source,
            "node-intermediate signing failed (root key unavailable); node bootstrap refusing to \
             start (no half-provisioned state)",
        );
        CaBootError::Ca { source }
    })?;

    // Seal the intermediate private signing key (PEM bytes) under the KEK, with
    // the distinct intermediate HKDF `info` label so the subkey is
    // domain-separated from the root subkey. This is what makes the intermediate
    // STABLE across restart.
    let key_pem = intermediate.signing_key().as_pem().as_bytes();
    let record = codec
        .seal_intermediate(kek, kek_id, key_pem)
        .map_err(|source| CaBootError::Ca { source })?;
    let envelope_bytes = record.archive_for_store()?;

    // Persist strictly AFTER a successful signature + seal, so the failure paths
    // above leave the IntentStore untouched (no half-provisioned state). The key
    // envelope and the public cert material are both written.
    intent.put(NODE_INTERMEDIATE_KEY_ENVELOPE_KEY, envelope_bytes.as_ref()).await?;
    intent
        .put(NODE_INTERMEDIATE_MATERIAL_KEY, &encode_intermediate_material(&intermediate))
        .await?;

    Ok(intermediate)
}

/// Subsequent boot: decrypt the persisted intermediate-key envelope
/// (Earned-Trust probe), reconstruct the SAME intermediate identity from the
/// persisted public cert material, and re-seed the CA adapter with it so
/// issuance signs under the persisted intermediate rather than a freshly-minted
/// ephemeral one (the chain-break fix).
#[allow(
    clippy::too_many_arguments,
    reason = "boot seam threads KEK + codec + redb path like boot_ca"
)]
async fn load_persistent_intermediate(
    ca: &dyn Ca,
    node: &NodeId,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    intent: &Arc<dyn IntentStore>,
    redb_path: &std::path::Path,
    envelope_bytes: &[u8],
) -> Result<IntermediateHandle, CaBootError> {
    let record: RootCaKeyRecord = RootCaKeyRecord::from_store_bytes(
        envelope_bytes,
        redb_path,
        Some("ca/node/intermediate-key-envelope/v1"),
    )?;

    // Earned-Trust probe: the persisted envelope MUST decrypt under the resolved
    // KEK. A tampered / wrong-KEK / corrupt record refuses startup with NO silent
    // re-issue (a re-issue would orphan every SVID signed under the persisted
    // intermediate).
    let key_pem_bytes = codec.open(kek, kek_id, &record).map_err(|source| {
        tracing::error!(
            name: "health.startup.refused",
            kek_id = %kek_id,
            cause = %source,
            "persisted node-intermediate key envelope failed to decrypt; node bootstrap refusing \
             to start (no silent re-issue)",
        );
        CaBootError::EnvelopeDecrypt { source }
    })?;

    // The decrypted key is the proof of trust; the persisted public cert
    // material carries the byte-identical intermediate identity to present on
    // reuse.
    let cert_material =
        intent.get(NODE_INTERMEDIATE_MATERIAL_KEY).await?.ok_or_else(|| CaBootError::Ca {
            source: CaError::signing_failed(
                "node-intermediate key envelope present but public cert material missing from \
                 IntentStore",
            ),
        })?;

    let handle = decode_intermediate_material(&cert_material, key_pem_bytes)?;

    // Re-seed the CA adapter with the persisted intermediate BEFORE any issuance.
    // A fresh adapter (e.g. `RcgenCa` with an empty cache) would otherwise mint a
    // new ephemeral intermediate on its first signing call, and every SVID signed
    // under the prior boot's intermediate would fail to chain to the refreshed
    // trust bundle (the chain-break fix). Idempotent for the same intermediate;
    // fails loud if a divergent intermediate was already minted
    // (issuance-before-adoption — see `Ca::adopt_persisted_intermediate`).
    ca.adopt_persisted_intermediate(node, &handle).map_err(|source| CaBootError::Ca { source })?;

    Ok(handle)
}

/// Newline-framed encoding of the public node-intermediate cert material: PEM,
/// DER (base16), serial. All three are public — no secret is persisted here.
fn encode_intermediate_material(intermediate: &IntermediateHandle) -> Vec<u8> {
    let der_hex = intermediate.cert_der().as_der().iter().fold(
        String::with_capacity(intermediate.cert_der().as_der().len() * 2),
        |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        },
    );
    let pem = intermediate.cert_pem().as_pem();
    format!(
        "{pem_len}\n{pem}{der_hex}\n{serial}\n",
        pem_len = pem.len(),
        pem = pem,
        der_hex = der_hex,
        serial = intermediate.serial(),
    )
    .into_bytes()
}

/// First boot: mint the root, seal its key under the KEK, persist both the
/// sealed envelope and the public cert material.
async fn generate_and_persist_root(
    ca: &dyn Ca,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    intent: &Arc<dyn IntentStore>,
) -> Result<RootCaHandle, CaBootError> {
    let root = ca.root().map_err(|source| CaBootError::Ca { source })?;

    // Seal the root private key (PEM bytes) under the KEK.
    let key_pem = root.signing_key().as_pem().as_bytes();
    let record = codec.seal(kek, kek_id, key_pem).map_err(|source| CaBootError::Ca { source })?;
    let envelope_bytes = record.archive_for_store()?;

    intent.put(ROOT_KEY_ENVELOPE_KEY, envelope_bytes.as_ref()).await?;
    intent.put(ROOT_CERT_MATERIAL_KEY, &encode_cert_material(&root)).await?;

    Ok(root)
}

/// Subsequent boot: decrypt the persisted envelope (Earned-Trust probe b),
/// reconstruct the SAME root identity from the persisted public cert material,
/// and re-seed the CA adapter with it so issuance signs under the persisted
/// root rather than a freshly-minted ephemeral one (the chain-break fix).
async fn load_persistent_root(
    ca: &dyn Ca,
    kek: &dyn Kek,
    kek_id: &KekId,
    codec: &RootKeyAeadCodec,
    intent: &Arc<dyn IntentStore>,
    redb_path: &std::path::Path,
    envelope_bytes: &[u8],
) -> Result<RootCaHandle, CaBootError> {
    let record: RootCaKeyRecord = RootCaKeyRecord::from_store_bytes(
        envelope_bytes,
        redb_path,
        Some("ca/root/key-envelope/v1"),
    )?;

    // Earned-Trust probe (b): the persisted envelope MUST decrypt under the
    // resolved KEK. A tampered / wrong-KEK / corrupt record refuses startup
    // with NO silent re-mint.
    let key_pem_bytes = codec.open(kek, kek_id, &record).map_err(|source| {
        tracing::error!(
            name: "health.startup.refused",
            kek_id = %kek_id,
            cause = %source,
            "persisted root-key envelope failed to decrypt; control-plane refusing to start (no \
             silent re-mint)",
        );
        CaBootError::EnvelopeDecrypt { source }
    })?;

    // The decrypted key is the proof of trust; the persisted public cert
    // material carries the byte-identical root identity to present on reuse.
    let cert_material =
        intent.get(ROOT_CERT_MATERIAL_KEY).await?.ok_or_else(|| CaBootError::Ca {
            source: CaError::signing_failed(
                "root-key envelope present but public cert material missing from IntentStore",
            ),
        })?;

    let handle = decode_cert_material(&cert_material, key_pem_bytes)?;

    // Re-seed the CA adapter with the persisted root BEFORE any issuance. A
    // fresh adapter (e.g. `RcgenCa` with an empty cache) would otherwise mint a
    // new ephemeral root on its first signing call, and nothing signed under it
    // would chain to the persisted anchor relying parties pin (the chain-break
    // fix). Idempotent for the same root; fails loud if a divergent root was
    // already minted (issuance-before-adoption — see `Ca::adopt_persisted_root`).
    ca.adopt_persisted_root(&handle).map_err(|source| CaBootError::Ca { source })?;

    Ok(handle)
}

/// Newline-framed encoding of the public root cert material: PEM, DER (base16),
/// serial. The PEM/DER/serial are public; no secret is persisted here.
fn encode_cert_material(root: &RootCaHandle) -> Vec<u8> {
    let der_hex = root.cert_der().as_der().iter().fold(
        String::with_capacity(root.cert_der().as_der().len() * 2),
        |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        },
    );
    // Field order: PEM (single base64-ish block — may contain newlines), so we
    // length-prefix the PEM and put DER-hex + serial on dedicated trailing
    // lines. Encode PEM length as the first line to avoid newline ambiguity.
    let pem = root.cert_pem().as_pem();
    format!(
        "{pem_len}\n{pem}{der_hex}\n{serial}\n",
        pem_len = pem.len(),
        pem = pem,
        der_hex = der_hex,
        serial = root.serial(),
    )
    .into_bytes()
}

/// Decode the newline-framed public cert material + decrypted key into a
/// [`RootCaHandle`] presenting the byte-identical root identity.
fn decode_cert_material(bytes: &[u8], key_pem_bytes: Vec<u8>) -> Result<RootCaHandle, CaBootError> {
    let (pem, der, serial) = decode_framed_material(bytes)?;

    // The decrypted key is PEM-shaped sign-capability material (the root
    // signing key was sealed in PEM form, not DER); the identity assertion is
    // on the public cert (cert_pem / cert_der / serial), which is
    // byte-identical to first boot.
    let key_pem = String::from_utf8(key_pem_bytes).map_err(|_| CaBootError::Ca {
        source: CaError::signing_failed("decrypted root key is not valid UTF-8 PEM"),
    })?;

    Ok(RootCaHandle::new(CaCertPem::new(pem), CaCertDer::new(der), serial, CaKeyPem::new(key_pem)))
}

/// Decode the newline-framed public node-intermediate cert material + decrypted
/// key into an [`IntermediateHandle`] presenting the byte-identical intermediate
/// identity. Mirrors [`decode_cert_material`] — the encoding is the same
/// newline-framed PEM/DER-hex/serial shape — but yields the intermediate handle.
fn decode_intermediate_material(
    bytes: &[u8],
    key_pem_bytes: Vec<u8>,
) -> Result<IntermediateHandle, CaBootError> {
    let (pem, der, serial) = decode_framed_material(bytes)?;

    // The decrypted key is PEM-shaped sign-capability material (the intermediate
    // signing key was sealed in PEM form, not DER); the identity assertion is on
    // the public cert (cert_pem / cert_der / serial), byte-identical to first
    // boot.
    let key_pem = String::from_utf8(key_pem_bytes).map_err(|_| CaBootError::Ca {
        source: CaError::signing_failed("decrypted node-intermediate key is not valid UTF-8 PEM"),
    })?;

    Ok(IntermediateHandle::new(
        CaCertPem::new(pem),
        CaCertDer::new(der),
        serial,
        CaKeyPem::new(key_pem),
    ))
}

/// Parse the shared newline-framed public cert material (PEM length-prefixed,
/// then DER-hex + serial on trailing lines) into its `(pem, der, serial)` parts.
/// The single parse site for both [`decode_cert_material`] (root) and
/// [`decode_intermediate_material`] (intermediate) — the two encoders emit the
/// identical frame shape, so the two decoders cannot drift on the framing.
fn decode_framed_material(bytes: &[u8]) -> Result<(String, Vec<u8>, CertSerial), CaBootError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CaBootError::Ca {
        source: CaError::signing_failed("persisted cert material is not valid UTF-8"),
    })?;
    let malformed = || CaBootError::Ca {
        source: CaError::signing_failed("persisted cert material is malformed"),
    };

    let (pem_len_str, rest) = text.split_once('\n').ok_or_else(malformed)?;
    let pem_len: usize = pem_len_str.parse().map_err(|_| malformed())?;
    if rest.len() < pem_len {
        return Err(malformed());
    }
    let (pem, tail) = rest.split_at(pem_len);
    let mut tail_lines = tail.lines();
    let der_hex = tail_lines.next().ok_or_else(malformed)?;
    let serial_str = tail_lines.next().ok_or_else(malformed)?;

    let der = decode_hex(der_hex).ok_or_else(malformed)?;
    let serial = CertSerial::new(serial_str).map_err(|_| malformed())?;
    Ok((pem.to_string(), der, serial))
}

/// Decode a lowercase-hex string to bytes; `None` on any non-hex / odd length.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()
}
