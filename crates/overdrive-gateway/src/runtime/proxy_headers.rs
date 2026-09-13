//! Pure request/response proxy-header rewriting.
//!
//! SCAFFOLD: true. Both production functions remain RED until DELIVER.
//! Source-local properties implement S-PIG-14 through S-PIG-16.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::net::Ipv4Addr;

use hyper::HeaderMap;
use overdrive_core::public_ingress::PublicHostname;

#[must_use]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by the public runtime step")
)]
pub(crate) fn rewrite_request_headers(
    _headers: HeaderMap,
    _accepted_client: Ipv4Addr,
    _authority: &PublicHostname,
) -> HeaderMap {
    todo!("SCAFFOLD: rewrite_request_headers")
}

#[must_use]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by the public runtime step")
)]
pub(crate) fn rewrite_response_headers(_headers: HeaderMap) -> HeaderMap {
    todo!("SCAFFOLD: rewrite_response_headers")
}

#[cfg(test)]
mod properties {
    use hyper::header::{CONNECTION, FORWARDED, HOST, HeaderName, HeaderValue, VIA};
    use proptest::prelude::*;

    use super::*;

    fn arb_end_to_end_header() -> impl Strategy<Value = (String, String)> {
        ("x-keep-[a-z]{1,8}", "[a-zA-Z0-9._ -]{1,24}")
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER request header rewrite step"]
        fn request_rewrite_preserves_all_unnominated_end_to_end_values(
            preserved in proptest::collection::vec(arb_end_to_end_header(), 0..12),
        ) {
            let mut headers = HeaderMap::new();
            for (name, value) in &preserved {
                headers.append(
                    HeaderName::from_bytes(name.as_bytes()).expect("generated name"),
                    HeaderValue::from_str(value).expect("generated visible value"),
                );
            }
            headers.insert(HOST, "attacker.example".parse().expect("header"));
            headers.append(FORWARDED, "for=198.51.100.7".parse().expect("header"));
            headers.append(HeaderName::from_static("x-forwarded-for"), "198.51.100.7".parse().expect("header"));
            headers.insert(CONNECTION, "keep-alive, x-hop".parse().expect("header"));
            headers.insert(HeaderName::from_static("x-hop"), "remove-me".parse().expect("header"));

            let authority = PublicHostname::new("api.example.com").expect("valid hostname");
            let rewritten = rewrite_request_headers(headers, Ipv4Addr::new(203, 0, 113, 9), &authority);

            for (name, value) in preserved {
                prop_assert!(
                    rewritten.get_all(name.as_str()).iter().any(|candidate| candidate.as_bytes() == value.as_bytes()),
                    "end-to-end header {name}={value:?} must survive",
                );
            }
            prop_assert_eq!(rewritten.get(HOST).expect("canonical Host"), "api.example.com");
            prop_assert!(rewritten.get("x-forwarded-for").is_none());
            prop_assert!(rewritten.get("x-hop").is_none());
            prop_assert_eq!(
                rewritten.get_all(VIA).iter().next_back().expect("Via"),
                "1.1 overdrive"
            );
            prop_assert_eq!(
                rewritten.get(FORWARDED).expect("Forwarded"),
                "for=203.0.113.9;proto=https;host=\"api.example.com\"",
            );
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER response header rewrite step"]
        fn response_rewrite_removes_connection_nominated_headers_and_appends_via(
            nominated in "[a-z](?:[a-z0-9-]{0,10}[a-z0-9])?",
            value in "[a-zA-Z0-9._ -]{1,24}",
        ) {
            prop_assume!(nominated != "via" && nominated != "connection");
            let name = HeaderName::from_bytes(nominated.as_bytes()).expect("generated header name");
            let mut headers = HeaderMap::new();
            headers.insert(CONNECTION, nominated.parse().expect("token"));
            headers.insert(name.clone(), HeaderValue::from_str(&value).expect("value"));
            headers.append(VIA, "1.0 prior".parse().expect("Via"));

            let rewritten = rewrite_response_headers(headers);
            prop_assert!(rewritten.get(name).is_none());
            let via: Vec<_> = rewritten.get_all(VIA).iter().collect();
            prop_assert_eq!(via.len(), 2);
            prop_assert_eq!(via[0], "1.0 prior");
            prop_assert_eq!(via[1], "1.1 overdrive");
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER fixed hop-by-hop header step"]
    fn fixed_hop_by_hop_headers_are_removed_on_both_directions() {
        for name in [
            "connection",
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
        ] {
            let mut request = HeaderMap::new();
            request.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("name"),
                "x".parse().expect("value"),
            );
            request.insert(HOST, "api.example.com".parse().expect("Host"));
            let authority = PublicHostname::new("api.example.com").expect("hostname");
            assert!(
                rewrite_request_headers(request, Ipv4Addr::LOCALHOST, &authority)
                    .get(name)
                    .is_none(),
                "{name} must be removed from requests",
            );

            let mut response = HeaderMap::new();
            response.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("name"),
                "x".parse().expect("value"),
            );
            assert!(
                rewrite_response_headers(response).get(name).is_none(),
                "{name} must be removed from responses",
            );
        }
    }
}
