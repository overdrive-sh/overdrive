//! Pure request/response proxy-header rewriting.
//!
//! Source-local properties implement S-PIG-14 through S-PIG-16.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::net::Ipv4Addr;

use http::HeaderMap;
use http::header::{
    CONNECTION, FORWARDED, HOST, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION,
    TE, TRAILER, TRANSFER_ENCODING, UPGRADE, VIA,
};
use overdrive_core::public_ingress::PublicHostname;

/// Headers that are hop-by-hop on every HTTP/1.1 message, `Connection` included.
const FIXED_HOP_BY_HOP: [HeaderName; 8] = [
    CONNECTION,
    HeaderName::from_static("keep-alive"),
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
];

/// Prefix of every removed `X-Forwarded-*` spelling (lowercase header names).
const X_FORWARDED_PREFIX: &str = "x-forwarded-";

/// The value this gateway appends to `Via`.
const VIA_VALUE: HeaderValue = HeaderValue::from_static("1.1 overdrive");

/// Rewrite the received request headers for the upstream: hop-by-hop and
/// `Connection`-nominated headers are removed, `Host` is the validated
/// authority, every received forwarding state is replaced by one approved
/// `Forwarded` value, and `Via` ends with this gateway.
#[must_use]
pub(crate) fn rewrite_request_headers(
    mut headers: HeaderMap,
    accepted_client: Ipv4Addr,
    authority: &PublicHostname,
) -> HeaderMap {
    strip_hop_by_hop(&mut headers);
    let forwarding: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().starts_with(X_FORWARDED_PREFIX))
        .cloned()
        .collect();
    for name in forwarding {
        headers.remove(name);
    }
    headers.remove(FORWARDED);
    if let Ok(host) = HeaderValue::from_str(authority.as_str()) {
        headers.insert(HOST, host);
    }
    let approved = format!("for={accepted_client};proto=https;host=\"{authority}\"");
    if let Ok(forwarded) = HeaderValue::from_str(&approved) {
        headers.insert(FORWARDED, forwarded);
    }
    headers.append(VIA, VIA_VALUE);
    headers
}

/// Rewrite the upstream response headers: hop-by-hop and
/// `Connection`-nominated headers are removed and `Via` ends with this gateway.
#[must_use]
pub(crate) fn rewrite_response_headers(mut headers: HeaderMap) -> HeaderMap {
    strip_hop_by_hop(&mut headers);
    headers.append(VIA, VIA_VALUE);
    headers
}

/// Remove every header `Connection` nominates (tokens that are not valid header
/// names nominate nothing), then the fixed hop-by-hop set.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let nominated: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|list| list.split(','))
        .filter_map(|token| HeaderName::from_bytes(token.trim().as_bytes()).ok())
        .collect();
    for name in nominated.iter().chain(FIXED_HOP_BY_HOP.iter()) {
        headers.remove(name);
    }
}

#[cfg(test)]
mod properties {
    use std::collections::BTreeMap;

    use http::header::{CONNECTION, CONTENT_TYPE, FORWARDED, HOST, HeaderName, HeaderValue, VIA};
    use proptest::prelude::*;

    use super::*;

    const CLIENT: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 9);
    const FORWARDED_VALUE: &str = "for=203.0.113.9;proto=https;host=\"api.example.com\"";
    const FIXED_HOP_BY_HOP: [&str; 8] = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];

    fn authority() -> PublicHostname {
        PublicHostname::new("api.example.com").expect("valid hostname")
    }

    fn header(name: &str, value: &str) -> (HeaderName, HeaderValue) {
        (
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        )
    }

    fn values(headers: &HeaderMap, name: &str) -> Vec<String> {
        headers
            .get_all(name)
            .iter()
            .map(|value| value.to_str().expect("visible value").to_owned())
            .collect()
    }

    fn arb_end_to_end_header() -> impl Strategy<Value = (String, String)> {
        ("x-keep-[a-z]{1,8}", "[a-zA-Z0-9._ -]{1,24}")
    }

    /// Value sequence per name, in arrival order.
    fn sequences(entries: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in entries {
            by_name.entry(name.clone()).or_default().push(value.clone());
        }
        by_name
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-14: unnominated end-to-end headers keep every value in order;
        /// the request carries exactly one canonical Host, the one approved
        /// Forwarded value and `Via: 1.1 overdrive` last.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn request_rewrite_preserves_all_unnominated_end_to_end_values(
            preserved in proptest::collection::vec(arb_end_to_end_header(), 0..12),
        ) {
            let mut headers = HeaderMap::new();
            for (name, value) in &preserved {
                let (name, value) = header(name, value);
                headers.append(name, value);
            }
            headers.insert(HOST, "attacker.example".parse().expect("header"));
            headers.append(FORWARDED, "for=198.51.100.7".parse().expect("header"));
            headers.append(FORWARDED, "for=198.51.100.8".parse().expect("header"));
            headers.append(HeaderName::from_static("x-forwarded-for"), "198.51.100.7".parse().expect("header"));
            headers.append(HeaderName::from_static("x-forwarded-proto"), "http".parse().expect("header"));
            headers.insert(CONNECTION, "keep-alive, x-hop".parse().expect("header"));
            headers.insert(HeaderName::from_static("x-hop"), "remove-me".parse().expect("header"));
            headers.append(VIA, "1.0 earlier-proxy".parse().expect("header"));

            let rewritten = rewrite_request_headers(headers, CLIENT, &authority());

            for (name, expected) in sequences(&preserved) {
                prop_assert_eq!(values(&rewritten, &name), expected, "end-to-end header {} must survive in order", name);
            }
            prop_assert_eq!(values(&rewritten, "host"), vec!["api.example.com".to_owned()]);
            prop_assert_eq!(values(&rewritten, "forwarded"), vec![FORWARDED_VALUE.to_owned()]);
            prop_assert!(rewritten.get("x-forwarded-for").is_none());
            prop_assert!(rewritten.get("x-forwarded-proto").is_none());
            prop_assert!(rewritten.get("x-hop").is_none());
            prop_assert!(rewritten.get(CONNECTION).is_none());
            prop_assert_eq!(
                values(&rewritten, "via"),
                vec!["1.0 earlier-proxy".to_owned(), "1.1 overdrive".to_owned()],
                "prior Via values keep their order and the Overdrive value is last",
            );
        }

        /// S-PIG-15: a response header nominated by `Connection` (any case,
        /// any list position) is removed, other end-to-end headers keep
        /// their value sequence, and `Via: 1.1 overdrive` is appended last.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn response_rewrite_removes_connection_nominated_headers_and_appends_via(
            nominated in "[a-z](?:[a-z0-9-]{0,10}[a-z0-9])?",
            value in "[a-zA-Z0-9._ -]{1,24}",
            preserved in proptest::collection::vec(arb_end_to_end_header(), 0..8),
        ) {
            prop_assume!(nominated != "via" && nominated != "connection");
            let mut headers = HeaderMap::new();
            for (name, value) in &preserved {
                let (name, value) = header(name, value);
                headers.append(name, value);
            }
            headers.insert(CONNECTION, format!("close ,  {}", nominated.to_ascii_uppercase()).parse().expect("token list"));
            let (name, nominated_value) = header(&nominated, &value);
            headers.insert(name.clone(), nominated_value);
            headers.append(VIA, "1.0 prior".parse().expect("Via"));

            let rewritten = rewrite_response_headers(headers);

            prop_assert!(rewritten.get(&name).is_none(), "nominated header must be removed");
            prop_assert!(rewritten.get(CONNECTION).is_none());
            for (kept, expected) in sequences(&preserved) {
                if kept != nominated {
                    prop_assert_eq!(values(&rewritten, &kept), expected);
                }
            }
            prop_assert_eq!(
                values(&rewritten, "via"),
                vec!["1.0 prior".to_owned(), "1.1 overdrive".to_owned()],
            );
        }
    }

    /// S-PIG-14: without a received Host the canonical Host is inserted; every
    /// `X-Forwarded-*` spelling and every `Forwarded` value is replaced by
    /// the single approved `Forwarded`; `Connection` naming `Via` removes the
    /// prior `Via` and the Overdrive value still appears exactly once.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn request_rewrite_replaces_host_and_forwarding_state_and_honours_connection_via() {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("x-forwarded-for", "198.51.100.7"),
            ("x-forwarded-host", "attacker.example"),
            ("x-forwarded-port", "8443"),
            ("x-forwarded-", "empty-suffix"),
            ("x-forwarded-for-extra", "198.51.100.9"),
            ("x-forward", "kept: not an X-Forwarded-* field"),
            ("content-type", "application/json"),
        ] {
            let (name, value) = header(name, value);
            headers.append(name, value);
        }
        headers.append(FORWARDED, "for=198.51.100.7".parse().expect("header"));
        headers.insert(CONNECTION, "via".parse().expect("header"));
        headers.append(VIA, "1.0 earlier-proxy".parse().expect("header"));

        let rewritten = rewrite_request_headers(headers, CLIENT, &authority());

        assert_eq!(values(&rewritten, "host"), ["api.example.com"], "Host inserted when absent");
        assert_eq!(values(&rewritten, "forwarded"), [FORWARDED_VALUE]);
        for removed in [
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-port",
            "x-forwarded-",
            "x-forwarded-for-extra",
        ] {
            assert!(rewritten.get(removed).is_none(), "{removed} must be removed");
        }
        assert_eq!(values(&rewritten, "x-forward"), ["kept: not an X-Forwarded-* field"]);
        assert_eq!(values(&rewritten, "content-type"), ["application/json"]);
        assert_eq!(
            values(&rewritten, "via"),
            ["1.1 overdrive"],
            "Connection: via removes the prior Via, ours is exactly once",
        );
        assert!(rewritten.get(CONNECTION).is_none());
    }

    /// S-PIG-14: the Forwarded value names the accepted IPv4 client and the
    /// validated hostname, not any received value.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn request_rewrite_forwarded_value_follows_the_accepted_client_and_authority() {
        let other = PublicHostname::new("www.example.org").expect("hostname");
        let rewritten =
            rewrite_request_headers(HeaderMap::new(), Ipv4Addr::new(192, 0, 2, 200), &other);
        assert_eq!(
            values(&rewritten, "forwarded"),
            ["for=192.0.2.200;proto=https;host=\"www.example.org\""],
        );
        assert_eq!(values(&rewritten, "host"), ["www.example.org"]);
        assert_eq!(values(&rewritten, "via"), ["1.1 overdrive"]);
    }

    /// S-PIG-15: response Via handling: prior values are kept in order, a
    /// `Connection`-nominated `Via` is removed first, a malformed Connection
    /// token names no header while valid tokens in the same list still do,
    /// and the Overdrive value is always last.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn response_rewrite_via_ordering_and_malformed_connection_tokens() {
        let mut keeps_via = HeaderMap::new();
        keeps_via.append(VIA, "1.0 first".parse().expect("Via"));
        keeps_via.append(VIA, "1.1 second".parse().expect("Via"));
        keeps_via.insert(CONTENT_TYPE, "text/plain".parse().expect("header"));
        let rewritten = rewrite_response_headers(keeps_via);
        assert_eq!(values(&rewritten, "via"), ["1.0 first", "1.1 second", "1.1 overdrive"]);
        assert_eq!(values(&rewritten, "content-type"), ["text/plain"]);

        let mut nominates_via = HeaderMap::new();
        nominates_via.insert(CONNECTION, "Via".parse().expect("header"));
        nominates_via.append(VIA, "1.0 first".parse().expect("Via"));
        assert_eq!(
            values(&rewrite_response_headers(nominates_via), "via"),
            ["1.1 overdrive"],
            "a Connection-nominated Via is removed; the Overdrive value remains",
        );

        let mut malformed = HeaderMap::new();
        malformed.insert(CONNECTION, "bad token!, x-real,, ".parse().expect("header"));
        malformed.insert("x-real", "gone".parse().expect("header"));
        malformed.insert("x-other", "kept".parse().expect("header"));
        let rewritten = rewrite_response_headers(malformed);
        assert!(
            rewritten.get("x-real").is_none(),
            "valid token beside a malformed one is honoured"
        );
        assert_eq!(values(&rewritten, "x-other"), ["kept"], "a malformed token names no header");
        assert!(rewritten.get(CONNECTION).is_none(), "Connection itself is always removed");
    }

    /// S-PIG-16: the fixed hop-by-hop set, and every header a `Connection`
    /// value nominates, is removed on both directions while an end-to-end
    /// header survives.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn fixed_hop_by_hop_headers_are_removed_on_both_directions() {
        for name in FIXED_HOP_BY_HOP {
            let (header_name, value) = header(name, "x");

            let mut request = HeaderMap::new();
            request.insert(header_name.clone(), value.clone());
            request.insert(HOST, "api.example.com".parse().expect("Host"));
            request.insert(CONTENT_TYPE, "text/plain".parse().expect("header"));
            let rewritten = rewrite_request_headers(request, Ipv4Addr::LOCALHOST, &authority());
            assert!(rewritten.get(name).is_none(), "{name} must be removed from requests");
            assert_eq!(values(&rewritten, "content-type"), ["text/plain"]);

            let mut response = HeaderMap::new();
            response.insert(header_name, value);
            response.insert(CONTENT_TYPE, "text/plain".parse().expect("header"));
            let rewritten = rewrite_response_headers(response);
            assert!(rewritten.get(name).is_none(), "{name} must be removed from responses");
            assert_eq!(values(&rewritten, "content-type"), ["text/plain"]);
        }

        let mut request = HeaderMap::new();
        request.insert(CONNECTION, "x-private, X-Other".parse().expect("header"));
        request.insert("x-private", "1".parse().expect("header"));
        request.insert("x-other", "2".parse().expect("header"));
        request.insert("x-public", "3".parse().expect("header"));
        let rewritten = rewrite_request_headers(request, Ipv4Addr::LOCALHOST, &authority());
        assert!(rewritten.get("x-private").is_none() && rewritten.get("x-other").is_none());
        assert_eq!(values(&rewritten, "x-public"), ["3"]);

        let mut response = HeaderMap::new();
        response.insert(CONNECTION, "x-private, X-Other".parse().expect("header"));
        response.insert("x-private", "1".parse().expect("header"));
        response.insert("x-other", "2".parse().expect("header"));
        response.insert("x-public", "3".parse().expect("header"));
        let rewritten = rewrite_response_headers(response);
        assert!(rewritten.get("x-private").is_none() && rewritten.get("x-other").is_none());
        assert_eq!(values(&rewritten, "x-public"), ["3"]);
    }
}
