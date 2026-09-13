//! Embedded public-ingress gateway.
//!
//! SCAFFOLD: true. DISTILL establishes the exact public contracts and RED
//! acceptance properties; DELIVER replaces the scaffold bodies.

#![forbid(unsafe_code)]
#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::manual_let_else,
        reason = "test fixture preconditions and explicit wrong-success branches"
    )
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};

use overdrive_core::public_ingress::PublicCertifiedKeyId;
use thiserror::Error;

pub mod application;
pub mod certified_key;
pub mod connector;
pub mod demand;
pub mod frontend;
pub mod ports;
pub mod route_set;
pub mod runtime;

/// One field participating in all-or-none gateway enablement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayConfigField {
    Address,
    CertifiedKeyId,
    CertificateChainPath,
    PrivateKeyPath,
}

/// Closed gateway configuration validation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayConfigError {
    #[error("partial public ingress gateway enablement; missing {missing:?}")]
    PartialEnablement { missing: Vec<GatewayConfigField> },
}

/// Host-local manual public-certified-key inputs.
#[derive(Clone)]
pub struct ManualCertifiedKeyConfig {
    id: PublicCertifiedKeyId,
    #[expect(dead_code, reason = "DISTILL RED accessor is not implemented yet")]
    certificate_chain_path: PathBuf,
    #[expect(dead_code, reason = "DISTILL RED accessor is not implemented yet")]
    private_key_path: PathBuf,
}

impl ManualCertifiedKeyConfig {
    /// Construct the all-present manual input.
    pub fn new(_id: PublicCertifiedKeyId, _chain: PathBuf, _key: PathBuf) -> Self {
        todo!("SCAFFOLD: ManualCertifiedKeyConfig::new")
    }
    /// Configured certified-key identity.
    pub fn id(&self) -> &PublicCertifiedKeyId {
        todo!("SCAFFOLD: ManualCertifiedKeyConfig::id")
    }
    /// Absolute leaf-first certificate-chain file.
    pub fn certificate_chain_path(&self) -> &Path {
        todo!("SCAFFOLD: ManualCertifiedKeyConfig::certificate_chain_path")
    }
    /// Absolute PKCS#8 private-key file.
    pub fn private_key_path(&self) -> &Path {
        todo!("SCAFFOLD: ManualCertifiedKeyConfig::private_key_path")
    }
}

impl std::fmt::Debug for ManualCertifiedKeyConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManualCertifiedKeyConfig")
            .field("id", &self.id)
            .field("certificate_chain_path", &"[REDACTED]")
            .field("private_key_path", &"[REDACTED]")
            .finish()
    }
}

/// Fixed IPv4 TCP/443 gateway configuration.
#[derive(Clone)]
pub struct GatewayConfig {
    bind: SocketAddrV4,
    manual_certified_key: ManualCertifiedKeyConfig,
}

impl GatewayConfig {
    /// Bind the supplied IPv4 address on the fixed public port 443.
    pub fn new(_bind_address: Ipv4Addr, _key: ManualCertifiedKeyConfig) -> Self {
        todo!("SCAFFOLD: GatewayConfig::new")
    }
    /// Fixed public listener address.
    pub const fn bind(&self) -> SocketAddrV4 {
        self.bind
    }
    /// Manual certified-key inputs owned by the source boundary.
    pub fn manual_certified_key(&self) -> &ManualCertifiedKeyConfig {
        todo!("SCAFFOLD: GatewayConfig::manual_certified_key")
    }
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayConfig")
            .field("bind", &self.bind)
            .field("manual_certified_key", &self.manual_certified_key)
            .finish()
    }
}
