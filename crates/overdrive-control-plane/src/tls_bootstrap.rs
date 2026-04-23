//! Ephemeral CA + trust triple bootstrap per ADR-0010.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! On first `overdrive cluster init`:
//! - Mint a self-signed CA (P-256, `rcgen`).
//! - Mint a server leaf cert with SANs `127.0.0.1`, `::1`, `localhost`,
//!   and the host's own hostname.
//! - Mint a client leaf cert for CLI use.
//! - Write `~/.overdrive/config` with base64-encoded PEM CA / crt / key
//!   (the "trust triple").
//!
//! Re-running `cluster init` re-mints everything. No persisted CA key.

use std::path::Path;

use crate::error::ControlPlaneError;

/// Material produced by `mint_ephemeral_ca`. All fields are in-memory
/// PEM; callers write them to the trust triple via `write_trust_triple`.
///
/// SCAFFOLD: true
pub struct CaMaterial {
    pub ca_cert_pem: String,
    pub server_leaf_cert_pem: String,
    pub server_leaf_key_pem: String,
    pub client_leaf_cert_pem: String,
    pub client_leaf_key_pem: String,
}

/// Mint the ephemeral CA + server leaf + client leaf. Multi-SAN on the
/// server cert per ADR-0010.
///
/// SCAFFOLD: true
pub fn mint_ephemeral_ca() -> Result<CaMaterial, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Write the trust triple to `<config_dir>/.overdrive/config` in the
/// Talos-shape YAML per ADR-0010 §R2.
///
/// SCAFFOLD: true
pub fn write_trust_triple(
    _config_dir: &Path,
    _endpoint: &str,
    _material: &CaMaterial,
) -> Result<(), ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Load a `rustls::ServerConfig` from minted server material. Pure on
/// PEM inputs; no filesystem reads.
///
/// SCAFFOLD: true
pub fn load_server_tls_config(
    _material: &CaMaterial,
) -> Result<rustls::ServerConfig, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}
