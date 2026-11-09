//! Public-ingress gateway — the `overdrive serve` operator surface, driven
//! in-process (S-PIG-18..20).
//!
//! The four `--gateway-*` arguments are one all-or-none enablement set: the
//! complete set activates the gateway, the empty set leaves it disabled, and
//! every other combination is refused with the exact missing fields before any
//! startup effect (`ServeArgs::gateway_config`, the first operation of
//! `serve::run_with_adapters`). Tests parse real argv through `Cli`, move the
//! typed fields into `ServeArgs` as `main` does, and read the resulting
//! `Result<Option<GatewayConfig>, GatewayConfigError>`; no subprocess, no
//! socket, no file read (the named files do not exist).

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE metadata tags")]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};

use clap::Parser as _;
use clap::error::ErrorKind;
use overdrive_cli::cli::{Cli, Command};
use overdrive_cli::commands::serve::ServeArgs;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_gateway::{GatewayConfig, GatewayConfigError, GatewayConfigField};

const ADDRESS_FLAG: &str = "--gateway-address";
const KEY_ID_FLAG: &str = "--gateway-certified-key-id";
const CHAIN_FLAG: &str = "--gateway-certificate-chain";
const KEY_FLAG: &str = "--gateway-private-key";

/// The four gateway flags with one concrete operator value each, in the
/// order the missing-field list reports them.
const FLAGS: [(&str, &str, GatewayConfigField); 4] = [
    (ADDRESS_FLAG, "127.0.0.2", GatewayConfigField::Address),
    (KEY_ID_FLAG, "api-origin", GatewayConfigField::CertifiedKeyId),
    (CHAIN_FLAG, "/nonexistent/operator/chain.pem", GatewayConfigField::CertificateChainPath),
    (KEY_FLAG, "/nonexistent/operator/key.pem", GatewayConfigField::PrivateKeyPath),
];

/// Parse `overdrive serve` argv and move the four typed fields into
/// `ServeArgs`, exactly as `main` does.
fn serve_args_from(argv: &[&str]) -> Result<ServeArgs, clap::Error> {
    let mut full = vec!["overdrive", "serve"];
    full.extend_from_slice(argv);
    let cli = Cli::try_parse_from(full)?;
    let Command::Serve {
        bind,
        data_dir: _,
        gateway_address,
        gateway_certified_key_id,
        gateway_certificate_chain,
        gateway_private_key,
    } = cli.command
    else {
        panic!("`overdrive serve` must parse to Command::Serve");
    };
    Ok(ServeArgs {
        bind: bind.parse().expect("bind default parses"),
        data_dir: PathBuf::from("/nonexistent/operator/data"),
        config_dir: PathBuf::from("/nonexistent/operator/config"),
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        gateway_address,
        gateway_certified_key_id,
        gateway_certificate_chain,
        gateway_private_key,
    })
}

/// The argv for the flags selected by `mask` (bit `n` selects `FLAGS[n]`).
fn argv_for_mask(mask: u8) -> Vec<&'static str> {
    let mut argv = Vec::new();
    for (index, (flag, value, _)) in FLAGS.iter().enumerate() {
        if mask & (1 << index) != 0 {
            argv.push(*flag);
            argv.push(*value);
        }
    }
    argv
}

fn missing_of(error: &GatewayConfigError) -> &[GatewayConfigField] {
    match error {
        GatewayConfigError::PartialEnablement { missing } => missing,
    }
}

/// S-PIG-18 — the complete set enables the gateway on the supplied address and
/// fixed port 443, carrying the manual key inputs unchanged; no file is read.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn complete_gateway_flag_set_enables_the_gateway_on_the_supplied_address() {
    let args = serve_args_from(&argv_for_mask(0b1111)).expect("four flags parse");

    let config: GatewayConfig =
        args.gateway_config().expect("complete set is valid").expect("complete set enables");

    assert_eq!(config.bind(), SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 443));
    let key = config.manual_certified_key();
    assert_eq!(key.id().as_str(), "api-origin");
    assert_eq!(key.certificate_chain_path(), Path::new("/nonexistent/operator/chain.pem"));
    assert_eq!(key.private_key_path(), Path::new("/nonexistent/operator/key.pem"));
}

/// S-PIG-18 — no gateway flag leaves the gateway disabled.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn no_gateway_flags_leave_the_gateway_disabled() {
    let args = serve_args_from(&[]).expect("bare serve parses");

    let config = args.gateway_config().expect("the empty set is valid");

    assert!(config.is_none(), "no flags must leave the gateway disabled, got {config:?}");
}

/// S-PIG-19 — each gateway argument is a scalar: a second occurrence is an
/// argument conflict at the argv boundary, not a silent last-wins; a value
/// that is not an IPv4 address or a canonical certified-key ID is refused by
/// its typed parser there too.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn repeated_or_malformed_gateway_arguments_are_rejected_at_argv() {
    for (flag, value, _) in FLAGS {
        let error = serve_args_from(&[flag, value, flag, value])
            .expect_err(&format!("`{flag}` given twice must be rejected"));
        assert_eq!(
            error.kind(),
            ErrorKind::ArgumentConflict,
            "`{flag}` given twice must be an argument conflict, got {:?}",
            error.kind()
        );
    }

    for (flag, malformed) in [(ADDRESS_FLAG, "not-an-address"), (KEY_ID_FLAG, "Not A Key ID!")] {
        let error = serve_args_from(&[flag, malformed])
            .expect_err(&format!("`{flag} {malformed}` must be rejected by its typed parser"));
        assert_eq!(
            error.kind(),
            ErrorKind::ValueValidation,
            "`{flag} {malformed}` must fail typed parsing, got {:?}",
            error.kind()
        );
    }
}

/// S-PIG-20 — every one of the fourteen partial sets is refused, and the error
/// names exactly the absent fields in `Address, CertifiedKeyId,
/// CertificateChainPath, PrivateKeyPath` order; nothing about the supplied
/// fields appears in it.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn partial_gateway_flag_sets_name_exactly_the_missing_fields() {
    for mask in 1u8..0b1111 {
        let args = serve_args_from(&argv_for_mask(mask)).expect("partial argv still parses");

        let error = args
            .gateway_config()
            .expect_err(&format!("mask {mask:#06b} is a partial set and must be refused"));

        let expected: Vec<GatewayConfigField> = FLAGS
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) == 0)
            .map(|(_, (_, _, field))| *field)
            .collect();
        assert_eq!(
            missing_of(&error),
            expected.as_slice(),
            "mask {mask:#06b}: the error must name exactly the absent fields in order"
        );
        let rendered = error.to_string();
        for (_, value, _) in FLAGS {
            assert!(
                !rendered.contains(value),
                "mask {mask:#06b}: the error text must not echo supplied value `{value}`: {rendered}"
            );
        }
    }
}
