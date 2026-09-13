//! S-PIG-18 through S-PIG-20 — gateway operator configuration contracts.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use clap::Parser;
use overdrive_cli::cli::Cli;
use overdrive_cli::commands::serve::ServeArgs;
use overdrive_core::public_ingress::PublicCertifiedKeyId;
use overdrive_gateway::{GatewayConfigError, GatewayConfigField};

fn serve_args(mask: u8) -> ServeArgs {
    ServeArgs {
        bind: "127.0.0.1:0".parse::<SocketAddr>().expect("operator listener"),
        data_dir: PathBuf::from("/tmp/overdrive-data"),
        config_dir: PathBuf::from("/tmp/overdrive-config"),
        gateway_address: (mask & 0b0001 != 0).then_some(Ipv4Addr::LOCALHOST),
        gateway_certified_key_id: (mask & 0b0010 != 0)
            .then(|| PublicCertifiedKeyId::new("api-origin").expect("valid ID")),
        gateway_certificate_chain: (mask & 0b0100 != 0)
            .then(|| PathBuf::from("/run/credentials/api-origin-chain.pem")),
        gateway_private_key: (mask & 0b1000 != 0)
            .then(|| PathBuf::from("/run/credentials/api-origin-key.pem")),
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER serve argv step"]
fn operator_can_name_the_complete_public_gateway_input_on_serve() {
    let parsed = Cli::try_parse_from([
        "overdrive",
        "serve",
        "--gateway-address",
        "192.0.2.10",
        "--gateway-certified-key-id",
        "api-origin",
        "--gateway-certificate-chain",
        "/run/credentials/api-origin-chain.pem",
        "--gateway-private-key",
        "/run/credentials/api-origin-key.pem",
    ]);
    assert!(parsed.is_ok(), "the exact four public gateway flags must parse");
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER serve argv step"]
fn repeated_gateway_scalar_argument_is_rejected_as_an_argument_conflict() {
    let error = Cli::try_parse_from([
        "overdrive",
        "serve",
        "--gateway-address",
        "192.0.2.10",
        "--gateway-address",
        "192.0.2.11",
        "--gateway-certified-key-id",
        "api-origin",
        "--gateway-certificate-chain",
        "/run/credentials/api-origin-chain.pem",
        "--gateway-private-key",
        "/run/credentials/api-origin-key.pem",
    ])
    .expect_err("a scalar flag must occur at most once");
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER all-or-none gateway config step"]
fn every_partial_enablement_combination_names_exactly_the_missing_fields() {
    for mask in 0u8..=0b1111 {
        let result = serve_args(mask).gateway_config();
        match mask {
            0 => assert!(result.expect("all absent is valid").is_none()),
            0b1111 => {
                let config = result.expect("all present is valid").expect("enabled");
                assert_eq!(
                    config.bind(),
                    "127.0.0.1:443".parse::<std::net::SocketAddrV4>().expect("fixed bind")
                );
                assert_eq!(config.manual_certified_key().id().as_str(), "api-origin");
            }
            _ => {
                let mut missing = Vec::new();
                if mask & 0b0001 == 0 {
                    missing.push(GatewayConfigField::Address);
                }
                if mask & 0b0010 == 0 {
                    missing.push(GatewayConfigField::CertifiedKeyId);
                }
                if mask & 0b0100 == 0 {
                    missing.push(GatewayConfigField::CertificateChainPath);
                }
                if mask & 0b1000 == 0 {
                    missing.push(GatewayConfigField::PrivateKeyPath);
                }
                assert!(
                    matches!(
                        result,
                        Err(GatewayConfigError::PartialEnablement { missing: actual })
                            if actual == missing
                    ),
                    "mask {mask:04b}"
                );
            }
        }
    }
}
