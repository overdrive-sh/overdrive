//! `SimKek` — in-memory [`Kek`] adapter for DST and integration fixtures
//! (ADR-0063 D3/D6).
//!
//! The production KEK provider is `SystemdCredsKeyring` (`overdrive-host`)
//! over the Linux kernel keyring; `SimKek` is its pure in-process test
//! double. It maps `KekId → KekMaterial` from a `BTreeMap` populated at
//! construction — no keyring, no FFI, no `$CREDENTIALS_DIRECTORY`, no kernel
//! keyring key to leak across runs.
//!
//! It enforces the real provider's contract precondition (`Kek::resolve`):
//! an unregistered id resolves to [`KekError::NotFound`], **never** a
//! zero/default KEK — a silent default KEK would make the at-rest envelope
//! seal meaningless (ADR-0063 Earned Trust).
//!
//! # Why this lives in `overdrive-sim`, not `overdrive-testing`
//!
//! `SimKek` is a **pure in-process test double**, so per
//! `.claude/rules/development.md` § "Shared real-infra test fixtures" it
//! belongs with the other `Sim*` adapters here, NOT in `overdrive-testing`
//! (which is for real-OS fixtures shared across crates). Both consumers
//! (`overdrive-control-plane`, `overdrive-cli`) already carry `overdrive-sim`
//! as a dev-dependency, so injecting `SimKek` through `ServerConfig.kek` needs
//! no new dependency wiring and touches no kernel keyring.
//!
//! # Dependency discipline
//!
//! `SimKek` is constructed explicitly by the fixture (no production-binding
//! default). A boot site that needs a KEK under test injects
//! `Arc::new(SimKek::for_boot())` into `ServerConfig.kek`; one that forgets
//! it fails to compile (the field is mandatory — feature-delta § C1-AMEND).

use std::collections::BTreeMap;

use overdrive_core::ca::kek::{KEK_LEN, Kek, KekError, KekMaterial};
use overdrive_core::ca::root_key_envelope::KekId;

/// Deterministic fixture KEK bytes for the canonical single-node boot KEK.
///
/// Any non-zero constant suffices — the seal/open round-trip only requires
/// that the SAME material resolves on seal and on open within one fixture's
/// lifetime. A fixed constant keeps `SimKek::for_boot()` reproducible across
/// runs (DST determinism, K3) without drawing entropy.
const FIXTURE_BOOT_KEK_BYTES: [u8; KEK_LEN] = [0x5a; KEK_LEN];

/// In-memory [`Kek`] provider: resolves only the ids registered at
/// construction, [`KekError::NotFound`] for everything else.
#[derive(Debug, Clone, Default)]
pub struct SimKek {
    keys: BTreeMap<KekId, [u8; KEK_LEN]>,
}

impl SimKek {
    /// An empty provider that resolves nothing — every `resolve` is
    /// [`KekError::NotFound`]. Use [`with`](Self::with) to register ids.
    #[must_use]
    pub const fn new() -> Self {
        Self { keys: BTreeMap::new() }
    }

    /// A provider pre-loaded with the canonical control-plane boot KEK
    /// (`ca_boot::root_kek_id()` = `"overdrive-ca-root"`) bound to
    /// deterministic fixture material.
    ///
    /// This is the one-liner every `run_server` / `run_server_with_obs_and_driver`
    /// fixture injects through `ServerConfig.kek` so `boot_ca`'s KEK-resolve
    /// probe (a) succeeds hermetically — no env, no kernel keyring (feature-delta
    /// § C1-AMEND, crafter obligation C-3).
    #[must_use]
    pub fn for_boot() -> Self {
        Self::new().with(&root_boot_kek_id(), FIXTURE_BOOT_KEK_BYTES)
    }

    /// Register `kek_id → bytes`, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, kek_id: &KekId, bytes: [u8; KEK_LEN]) -> Self {
        self.keys.insert(kek_id.clone(), bytes);
        self
    }
}

impl Kek for SimKek {
    fn resolve(&self, kek_id: &KekId) -> Result<KekMaterial, KekError> {
        self.keys
            .get(kek_id)
            .map(|bytes| KekMaterial::new(*bytes))
            .ok_or_else(|| KekError::not_found(kek_id.clone()))
    }
}

/// The canonical single-node boot KEK identity (`"overdrive-ca-root"`),
/// matching `overdrive_control_plane::ca_boot::root_kek_id()`.
///
/// Re-derived here from the same literal rather than importing the
/// control-plane crate — `overdrive-sim` must not depend on
/// `overdrive-control-plane` (it sits below it in the dependency graph).
/// The literal is the stable, pinned KEK id (`root_key_envelope::KekId`).
fn root_boot_kek_id() -> KekId {
    KekId::new("overdrive-ca-root")
        .unwrap_or_else(|e| unreachable!("`overdrive-ca-root` is a valid KekId: {e}"))
}
