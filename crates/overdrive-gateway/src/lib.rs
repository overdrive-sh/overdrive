//! Embedded public-ingress gateway.
//!
//! SCAFFOLD: true. The data shapes, closed error enums and trivial accessors
//! below are the DESIGN-pinned contract; every behaviour body is DELIVER's and
//! fails with SCAFFOLD text. Acceptance bodies live in the source-local test
//! modules and in `tests/integration/public_ingress_gateway.rs`.

#![forbid(unsafe_code)]
#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    clippy::struct_field_names,
    clippy::needless_pass_by_ref_mut,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's and several pinned items have no live caller until then"
)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
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
#[derive(Debug, Error)]
pub enum GatewayConfigError {
    #[error("partial public ingress gateway enablement; missing {missing:?}")]
    PartialEnablement { missing: Vec<GatewayConfigField> },
}

/// Host-local manual public-certified-key inputs.
#[derive(Clone)]
pub struct ManualCertifiedKeyConfig {
    id: PublicCertifiedKeyId,
    certificate_chain_path: PathBuf,
    private_key_path: PathBuf,
}

impl ManualCertifiedKeyConfig {
    /// Construct the all-present manual input.
    pub fn new(id: PublicCertifiedKeyId, chain: PathBuf, key: PathBuf) -> Self {
        Self { id, certificate_chain_path: chain, private_key_path: key }
    }
    /// Configured certified-key identity.
    pub fn id(&self) -> &PublicCertifiedKeyId {
        &self.id
    }
    /// Absolute leaf-first certificate-chain file.
    pub fn certificate_chain_path(&self) -> &Path {
        &self.certificate_chain_path
    }
    /// Absolute PKCS#8 private-key file.
    pub fn private_key_path(&self) -> &Path {
        &self.private_key_path
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

/// Fixed IPv4 TCP/443 gateway configuration: the supplied address, port 443.
#[derive(Clone)]
pub struct GatewayConfig {
    bind: SocketAddrV4,
    manual_certified_key: ManualCertifiedKeyConfig,
}

impl GatewayConfig {
    /// Bind the supplied IPv4 address on the fixed public port 443.
    pub fn new(bind_address: Ipv4Addr, key: ManualCertifiedKeyConfig) -> Self {
        Self { bind: SocketAddrV4::new(bind_address, 443), manual_certified_key: key }
    }
    /// Public listener address; the port is always 443.
    pub const fn bind(&self) -> SocketAddrV4 {
        self.bind
    }
    /// Manual certified-key inputs owned by the source boundary.
    pub fn manual_certified_key(&self) -> &ManualCertifiedKeyConfig {
        &self.manual_certified_key
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
