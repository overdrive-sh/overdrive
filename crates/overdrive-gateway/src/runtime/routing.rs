//! Pure public-authority normalization.
//!
//! SCAFFOLD: true. The production function is DELIVER's.
//! Source-local properties implement S-PIG-12 and S-PIG-13.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use http::HeaderMap;
use overdrive_core::public_ingress::{PublicHostname, PublicHostnameError};
use thiserror::Error;

/// Closed public Host/SNI normalization failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
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

pub(crate) fn normalize_route_authority(
    _headers: &HeaderMap,
    _sni: &PublicHostname,
) -> Result<PublicHostname, RouteAuthorityError> {
    todo!("SCAFFOLD: normalize_route_authority")
}

#[cfg(test)]
mod properties {
    use http::header::{HOST, HeaderValue};
    use proptest::prelude::*;

    use super::*;

    fn host_headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, value.parse().expect("visible header value"));
        headers
    }

    fn sni() -> PublicHostname {
        PublicHostname::new("api.example.com").expect("valid hostname")
    }

    fn arb_hostname() -> impl Strategy<Value = String> {
        proptest::collection::vec("[a-z](?:[a-z0-9-]{0,6}[a-z0-9])?", 2..5)
            .prop_map(|labels| labels.join("."))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-12: the Host name is ASCII case-folded and an explicit port
        /// 443 is removed before the comparison with the TLS SNI.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER gateway-application-owner"]
        fn canonical_authority_accepts_ascii_case_and_optional_443(
            canonical in arb_hostname(),
            explicit_port in any::<bool>(),
            uppercase in any::<bool>(),
        ) {
            let host = if uppercase { canonical.to_ascii_uppercase() } else { canonical.clone() };
            let authority = if explicit_port { format!("{host}:443") } else { host };
            let headers = host_headers(&authority);
            let sni = PublicHostname::new(&canonical).expect("generated hostname");
            prop_assert_eq!(normalize_route_authority(&headers, &sni), Ok(sni));
        }

        /// S-PIG-13: any explicit port other than 443 is rejected with that
        /// exact port, never as an SNI mismatch.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER gateway-application-owner"]
        fn every_explicit_port_other_than_443_is_unsupported(
            port in any::<u16>().prop_filter("443 is the only accepted port", |port| *port != 443),
        ) {
            let headers = host_headers(&format!("api.example.com:{port}"));
            prop_assert_eq!(
                normalize_route_authority(&headers, &sni()),
                Err(RouteAuthorityError::UnsupportedPort { got: port }),
            );
        }

        /// S-PIG-13: a valid Host naming another hostname is the SNI mismatch
        /// and nothing else.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER gateway-application-owner"]
        fn a_different_valid_hostname_is_exactly_an_sni_mismatch(
            other in arb_hostname(),
            explicit_port in any::<bool>(),
        ) {
            prop_assume!(other != "api.example.com");
            let authority = if explicit_port { format!("{other}:443") } else { other };
            prop_assert_eq!(
                normalize_route_authority(&host_headers(&authority), &sni()),
                Err(RouteAuthorityError::SniMismatch),
            );
        }
    }

    /// S-PIG-13: every authority failure is its own variant: missing,
    /// duplicate, non-ASCII, malformed (including userinfo), wrong port,
    /// invalid hostname and SNI mismatch.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway-application-owner"]
    fn malformed_duplicate_wrong_port_and_sni_mismatch_are_distinct() {
        let sni = sni();

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

        let mut duplicate_conflicting = HeaderMap::new();
        duplicate_conflicting.append(HOST, "api.example.com".parse().expect("header"));
        duplicate_conflicting.append(HOST, "other.example.com".parse().expect("header"));
        assert_eq!(
            normalize_route_authority(&duplicate_conflicting, &sni),
            Err(RouteAuthorityError::DuplicateHost),
            "two Host values are rejected before any comparison",
        );

        assert_eq!(
            normalize_route_authority(&host_headers("api.example.com:8443"), &sni),
            Err(RouteAuthorityError::UnsupportedPort { got: 8443 }),
        );
        assert_eq!(
            normalize_route_authority(&host_headers("api.example.com:80"), &sni),
            Err(RouteAuthorityError::UnsupportedPort { got: 80 }),
        );
        assert_eq!(
            normalize_route_authority(&host_headers("other.example.com:8443"), &sni),
            Err(RouteAuthorityError::UnsupportedPort { got: 8443 }),
            "a wrong port is a 400 even when the name would also mismatch the SNI",
        );

        assert_eq!(
            normalize_route_authority(&host_headers("other.example.com"), &sni),
            Err(RouteAuthorityError::SniMismatch)
        );

        let mut non_ascii = HeaderMap::new();
        non_ascii.insert(
            HOST,
            HeaderValue::from_bytes(b"api\xff.example.com").expect("obs-text header"),
        );
        assert_eq!(
            normalize_route_authority(&non_ascii, &sni),
            Err(RouteAuthorityError::NonAsciiHost),
        );

        for malformed in [
            "api.example.com:80:90",
            "[::1",
            "user@api.example.com",
            "user:secret@api.example.com",
            "user@api.example.com:443",
        ] {
            assert_eq!(
                normalize_route_authority(&host_headers(malformed), &sni),
                Err(RouteAuthorityError::InvalidAuthority),
                "{malformed:?} is not a valid Host authority for the gateway",
            );
        }

        for (invalid, cause) in [
            ("-api.example.com", PublicHostnameError::InvalidLabel),
            ("api..example.com", PublicHostnameError::EmptyLabel),
            ("*.example.com", PublicHostnameError::Wildcard),
            ("203.0.113.9", PublicHostnameError::IpLiteral),
            ("api.example.com.", PublicHostnameError::TerminalDot),
        ] {
            assert_eq!(
                normalize_route_authority(&host_headers(invalid), &sni),
                Err(RouteAuthorityError::InvalidHostname(cause)),
                "{invalid:?} is an invalid public hostname",
            );
        }
    }
}
