//! Render functions for Service `[[listener]]` blocks.
//!
//! Per `workload-kind-discriminator` Slice 06 / ADR-0047 §1
//! (Service listener fields) + §8 of `distill/test-scenarios.md`
//! (S-08-07, S-08-08, S-08-12).
//!
//! The KPI K6 byte-equality property is structurally enforced by
//! sourcing the deferral URL from a single SSOT constant
//! ([`SERVICE_VIP_ALLOCATOR_TRACKING_URL`]) — both the submit echo
//! and the alloc-status surface read it; drift across surfaces is
//! structurally impossible.

use overdrive_core::aggregate::Listener;

/// Single SSOT for the runtime-VIP-allocator deferral tracking URL.
///
/// Per slice 06 spec the constant value is byte-equal to
/// `https://github.com/overdrive-sh/overdrive/issues/167`. KPI K6
/// asserts that both the submit echo (`Listeners:` section) and the
/// alloc-status `Listeners:` section emit the same URL by reading
/// from this constant — drift across surfaces is structurally
/// impossible.
///
/// The value is a `&'static str` so every reader sees the same
/// statically-allocated bytes; there is no constructor or
/// canonicalisation step that could re-derive a slightly different
/// form between call sites.
pub const SERVICE_VIP_ALLOCATOR_TRACKING_URL: &str =
    "https://github.com/overdrive-sh/overdrive/issues/167";

/// Render a single listener line. Per S-08-07 + S-08-12, the form is:
///
/// * Pinned VIP: `<vip>:<port>/<protocol>` (e.g. `10.0.0.1:8080/tcp`).
/// * Pending VIP: `(vip: pending allocation — see <URL>):<port>/<protocol>`
///   where `<URL>` is byte-equal to [`SERVICE_VIP_ALLOCATOR_TRACKING_URL`].
///
/// Protocol is rendered in lowercase canonical form via
/// [`overdrive_core::dataplane::backend_key::Proto`]'s `Display` impl
/// (which sources from `Proto::as_str`).
#[must_use]
pub fn format_listener_line(l: Listener) -> String {
    let port = l.port.get();
    let proto = l.protocol;
    let url = SERVICE_VIP_ALLOCATOR_TRACKING_URL;
    l.vip.map_or_else(
        || format!("(vip: pending allocation — see {url}):{port}/{proto}"),
        |vip| format!("{vip}:{port}/{proto}"),
    )
}

/// Render the `Listeners:` section emitted by both the Service submit
/// echo and the alloc-status output.
///
/// Per S-08-07 / S-08-08 / S-08-09 / S-08-12 the section is identical
/// across surfaces — the byte-equality property KPI K6 reads as: the
/// `Listeners:` section text in submit-echo equals the `Listeners:`
/// section text in alloc-status for the same Service spec.
///
/// Empty `listeners` produces an empty string (caller may suppress
/// the section entirely). The Service parser rejects zero-listener
/// Specs at parse time per S-08-03 — this fn handling the empty
/// case is purely defensive.
#[must_use]
pub fn format_listeners_section(listeners: &[Listener]) -> String {
    use std::fmt::Write as _;
    if listeners.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    let _ = writeln!(s, "Listeners:");
    for l in listeners {
        let _ = writeln!(s, "  {}", format_listener_line(*l));
    }
    s
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::expect_fun_call)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::num::NonZeroU16;

    use overdrive_core::aggregate::ServiceVip;
    use overdrive_core::dataplane::backend_key::Proto;

    use super::*;

    fn pinned(port: u16, proto: Proto, vip_octet: u8) -> Listener {
        Listener {
            port: NonZeroU16::new(port).expect("non-zero port"),
            protocol: proto,
            vip: Some(
                ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, vip_octet)))
                    .expect("IPv4 is always a valid ServiceVip"),
            ),
        }
    }

    fn pending(port: u16, proto: Proto) -> Listener {
        Listener { port: NonZeroU16::new(port).expect("non-zero port"), protocol: proto, vip: None }
    }

    /// S-08-12 — the deferral URL is sourced from the single SSOT
    /// constant and references issue #167.
    #[test]
    fn s_08_12_tracking_url_constant_references_issue_167() {
        assert_eq!(
            SERVICE_VIP_ALLOCATOR_TRACKING_URL,
            "https://github.com/overdrive-sh/overdrive/issues/167"
        );
    }

    /// S-08-07 — submit echo includes a Listeners section with both
    /// pinned and pending VIPs rendered correctly.
    #[test]
    fn s_08_07_pinned_listener_renders_as_vip_port_proto() {
        let l = pinned(8080, Proto::Tcp, 1);
        assert_eq!(format_listener_line(l), "10.0.0.1:8080/tcp");
    }

    #[test]
    fn s_08_07_pending_listener_uses_tracking_url_verbatim() {
        let l = pending(8081, Proto::Udp);
        let line = format_listener_line(l);
        assert!(line.contains("pending allocation"), "line: {line}");
        assert!(
            line.contains(SERVICE_VIP_ALLOCATOR_TRACKING_URL),
            "tracking URL must appear verbatim in: {line}"
        );
        assert!(line.ends_with(":8081/udp"), "port/proto suffix: {line}");
    }

    /// S-08-09 — byte-equality across surfaces. We exercise the
    /// section-render fn twice with the same input and assert the
    /// bytes match. (The same fn is the SSOT both surfaces call into,
    /// so this is structural.)
    #[test]
    fn s_08_09_listeners_section_is_byte_equal_across_calls() {
        let listeners = [pinned(8080, Proto::Tcp, 1), pending(8081, Proto::Udp)];
        let a = format_listeners_section(&listeners);
        let b = format_listeners_section(&listeners);
        assert_eq!(a.as_bytes(), b.as_bytes());
        assert!(a.contains("Listeners:"));
        assert!(a.contains("10.0.0.1:8080/tcp"));
        assert!(a.contains(":8081/udp"));
    }

    /// S-08-08 — every listener line's protocol is rendered in
    /// lowercase canonical form.
    #[test]
    fn s_08_08_protocol_renders_lowercase() {
        let l = pinned(443, Proto::Tcp, 5);
        let line = format_listener_line(l);
        assert!(line.ends_with("/tcp"), "lowercase: {line}");
        let l_udp = pending(53, Proto::Udp);
        let line_udp = format_listener_line(l_udp);
        assert!(line_udp.ends_with("/udp"), "lowercase: {line_udp}");
    }
}
