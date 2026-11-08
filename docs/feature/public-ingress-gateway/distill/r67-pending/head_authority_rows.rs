// R-67 / S-PIG-82 / S-PIG-142 (head rows). NOT IN THE BUILD.
//
// These bodies name `RequestHeadError::Authority(RouteAuthorityError)`, which does
// not exist until roadmap step 05-02 (key `request-authority-single-implementation`).
// The crafter, in the same commit that adds the variant and deletes `check_host` and
// the six `Host*` variants:
//   1. replaces the two existing tests in the `mod tests` of
//      `crates/overdrive-gateway/src/runtime/http1/head.rs`
//      (`every_request_head_error_maps_to_its_status` and
//      `target_and_host_rules_hold_at_their_boundaries`) with the two tests below,
//      verbatim (the doc comments and bodies are the acceptance designer's);
//   2. adds `use crate::runtime::routing::RouteAuthorityError;` and
//      `use overdrive_core::public_ingress::PublicHostnameError;` to that module.
// The existing helpers (`request_error`, `request_result`, `block`, `request_with`,
// `sni`) are reused unchanged.

    /// S-PIG-82 (R-67): `RequestHeadError::status` is the Request-rules table. Line
    /// violations, framing faults, a non-origin-form target, `CONNECT` and every
    /// `Authority(_)` error except the SNI mismatch are 400; `Authority(SniMismatch)`
    /// is 421; a declared length above the ceiling is 413; an oversized head or too
    /// many fields is 431; a version other than HTTP/1.1 is 505. One row per
    /// `RouteAuthorityError` variant.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_request_head_error_maps_to_its_status() {
        let table: [(RequestHeadError, u16); 19] = [
            (RequestHeadError::Line(LineViolation::Nul), 400),
            (RequestHeadError::Connect, 400),
            (RequestHeadError::NonOriginFormTarget, 400),
            (RequestHeadError::Authority(RouteAuthorityError::MissingHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::DuplicateHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::NonAsciiHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::InvalidAuthority), 400),
            (RequestHeadError::Authority(RouteAuthorityError::UnsupportedPort { got: 8443 }), 400),
            (
                RequestHeadError::Authority(RouteAuthorityError::InvalidHostname(
                    PublicHostnameError::InvalidLabel,
                )),
                400,
            ),
            (RequestHeadError::Authority(RouteAuthorityError::SniMismatch), 421),
            (RequestHeadError::Framing(FramingFault::BothLengthAndCoding), 400),
            (RequestHeadError::Framing(FramingFault::ContentLengthOverflow), 400),
            (RequestHeadError::DeclaredLengthAboveCeiling, 413),
            (RequestHeadError::HeadTooLarge, 431),
            (RequestHeadError::TooManyHeaders, 431),
            (RequestHeadError::VersionNotHttp11, 505),
            (RequestHeadError::Http10, 505),
            (RequestHeadError::Line(LineViolation::BareLf), 400),
            (RequestHeadError::Framing(FramingFault::UnsupportedCoding), 400),
        ];
        for (error, status) in table {
            assert_eq!(error.status().as_u16(), status, "{error:?}");
        }
    }

    /// S-PIG-82 (target and Host, R-67): a non-origin-form target and `CONNECT` are
    /// refused; every Host row is the `Authority(<RouteAuthorityError>)` the one
    /// `normalize_route_authority` produces: missing, duplicated, invalid, with
    /// userinfo, with a port other than 443 and with a non-canonical port spelling
    /// are 400; an explicit `:443` and the canonical Host are accepted; a Host that
    /// differs from the SNI is a 421.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn target_and_host_rules_hold_at_their_boundaries() {
        let host = |value: &str| block("GET /a HTTP/1.1", &[format!("Host: {value}")]);
        let authority = |value: &str| match request_error(&host(value)) {
            Some(RequestHeadError::Authority(error)) => error,
            other => panic!("Host {value:?}: expected an Authority error, got {other:?}"),
        };

        assert_eq!(request_result(&request_with("GET /a HTTP/1.1", &[])), Ok(BodyFraming::NoBody));
        assert_eq!(request_result(&host("api.example.com:443")), Ok(BodyFraming::NoBody));

        assert_eq!(
            request_error(&request_with("GET http://api.example.com/a HTTP/1.1", &[])),
            Some(RequestHeadError::NonOriginFormTarget),
        );
        assert_eq!(
            request_error(&request_with("GET * HTTP/1.1", &[])),
            Some(RequestHeadError::NonOriginFormTarget),
        );
        let connect = request_error(&block(
            "CONNECT api.example.com:443 HTTP/1.1",
            &["Host: api.example.com:443".to_owned()],
        ))
        .expect("CONNECT is refused");
        assert_eq!(connect.status(), StatusCode::BAD_REQUEST);

        assert_eq!(
            request_error(&block("GET /a HTTP/1.1", &[])),
            Some(RequestHeadError::Authority(RouteAuthorityError::MissingHost)),
        );
        assert_eq!(
            request_error(&block(
                "GET /a HTTP/1.1",
                &["Host: api.example.com".to_owned(), "Host: api.example.com".to_owned()],
            )),
            Some(RequestHeadError::Authority(RouteAuthorityError::DuplicateHost)),
        );
        assert!(matches!(authority("bad host!"), RouteAuthorityError::InvalidHostname(_)));
        assert_eq!(authority("user@api.example.com"), RouteAuthorityError::InvalidAuthority);
        assert_eq!(
            authority("api.example.com:8443"),
            RouteAuthorityError::UnsupportedPort { got: 8443 },
        );
        for spelling in [":+443", ":0443", ":00443", ":", ":4a3", ":65536"] {
            assert_eq!(
                authority(&format!("api.example.com{spelling}")),
                RouteAuthorityError::InvalidAuthority,
                "{spelling:?}",
            );
        }

        let mismatch = request_error(&host("other.example.com")).expect("Host differs from SNI");
        assert_eq!(mismatch, RequestHeadError::Authority(RouteAuthorityError::SniMismatch));
        assert_eq!(mismatch.status(), StatusCode::MISDIRECTED_REQUEST);
    }
