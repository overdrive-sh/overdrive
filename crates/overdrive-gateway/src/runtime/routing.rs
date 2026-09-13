//! Pure public-authority normalization.
//!
//! SCAFFOLD: true. The production function remains RED until DELIVER.
//! Source-local properties implement S-PIG-12 and S-PIG-13.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use hyper::HeaderMap;
use overdrive_core::public_ingress::{PublicHostname, PublicHostnameError};
use thiserror::Error;

/// Closed public Host/SNI normalization failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by the public runtime step")
)]
pub(crate) enum RouteAuthorityError {
    #[error("Host header is missing")]
    MissingHost,
    #[error("Host header occurs more than once")]
    DuplicateHost,
    #[error("Host header is not visible ASCII")]
    NonAsciiHost,
    #[error("Host header is not a valid HTTP authority")]
    InvalidAuthority,
    #[error("Host authority port must be 443, got {got}")]
    UnsupportedPort { got: u16 },
    #[error("Host name is invalid: {0}")]
    InvalidHostname(PublicHostnameError),
    #[error("Host does not equal the TLS SNI")]
    SniMismatch,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by the public runtime step")
)]
pub(crate) fn normalize_route_authority(
    _headers: &HeaderMap,
    _sni: &PublicHostname,
) -> Result<PublicHostname, RouteAuthorityError> {
    todo!("SCAFFOLD: normalize_route_authority")
}

#[cfg(test)]
mod properties {
    use hyper::header::HOST;
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER authority normalization step"]
        fn canonical_authority_accepts_ascii_case_and_optional_443(
            labels in proptest::collection::vec("[a-z](?:[a-z0-9-]{0,6}[a-z0-9])?", 2..5),
            explicit_port in any::<bool>(),
            uppercase in any::<bool>(),
        ) {
            let canonical = labels.join(".");
            let host = if uppercase { canonical.to_ascii_uppercase() } else { canonical.clone() };
            let authority = if explicit_port { format!("{host}:443") } else { host };
            let mut headers = HeaderMap::new();
            headers.insert(HOST, authority.parse().expect("visible ASCII authority"));
            let sni = PublicHostname::new(&canonical).expect("generated hostname");
            prop_assert_eq!(normalize_route_authority(&headers, &sni), Ok(sni));
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER authority normalization step"]
    fn malformed_duplicate_wrong_port_and_sni_mismatch_are_distinct() {
        let sni = PublicHostname::new("api.example.com").expect("valid hostname");

        assert_eq!(
            normalize_route_authority(&HeaderMap::new(), &sni),
            Err(RouteAuthorityError::MissingHost)
        );

        let mut duplicate = HeaderMap::new();
        duplicate.append(HOST, "api.example.com".parse().expect("header"));
        duplicate.append(HOST, "api.example.com".parse().expect("header"));
        assert_eq!(
            normalize_route_authority(&duplicate, &sni),
            Err(RouteAuthorityError::DuplicateHost)
        );

        let mut wrong_port = HeaderMap::new();
        wrong_port.insert(HOST, "api.example.com:8443".parse().expect("header"));
        assert_eq!(
            normalize_route_authority(&wrong_port, &sni),
            Err(RouteAuthorityError::UnsupportedPort { got: 8443 }),
        );

        let mut wrong_host = HeaderMap::new();
        wrong_host.insert(HOST, "other.example.com".parse().expect("header"));
        assert_eq!(
            normalize_route_authority(&wrong_host, &sni),
            Err(RouteAuthorityError::SniMismatch)
        );

        let mut non_ascii = HeaderMap::new();
        non_ascii.insert(
            HOST,
            hyper::header::HeaderValue::from_bytes(b"api\xff.example.com")
                .expect("obs-text header"),
        );
        assert_eq!(
            normalize_route_authority(&non_ascii, &sni),
            Err(RouteAuthorityError::NonAsciiHost),
        );

        let mut invalid_authority = HeaderMap::new();
        invalid_authority.insert(HOST, "api.example.com:".parse().expect("visible header"));
        assert_eq!(
            normalize_route_authority(&invalid_authority, &sni),
            Err(RouteAuthorityError::InvalidAuthority),
        );

        let mut invalid_hostname = HeaderMap::new();
        invalid_hostname.insert(HOST, "-api.example.com".parse().expect("visible header"));
        assert_eq!(
            normalize_route_authority(&invalid_hostname, &sni),
            Err(RouteAuthorityError::InvalidHostname(PublicHostnameError::InvalidLabel)),
        );
    }
}
