//! `[dataplane]` TOML config-section parser surface.
//!
//! Step 02-01 of `backend-discovery-bridge-service-reachability`
//! per architecture.md § 5.1. Owns the boot-time parser for the
//! operator-supplied required dataplane interface bindings:
//!
//! ```toml
//! [dataplane]
//! client_iface  = "lb_veth_a"
//! backend_iface = "lb_veth_b"
//! ```
//!
//! The `[dataplane.vip_allocator]` subsection lives at the same root
//! and is parsed by [`crate::vip_allocator_config`]; the two parsers
//! deserialise independently against the same TOML document via the
//! `[dataplane]` table wrapper, so adding fields here does not
//! interfere with the VIP-allocator parser.
//!
//! Missing-section policy mirrors the `[tls]` precedent per ADR-0010:
//! a missing `[dataplane]` section is a hard boot refusal with
//! [`crate::error::ControlPlaneError::Validation`], not the
//! "default-with-override" posture the `[dataplane.vip_allocator]`
//! parser uses for its subsection — the two interface bindings have
//! no safe default (production needs real `client_iface` /
//! `backend_iface` values per Phase 2.3 XDP attachment, and the
//! single-cut migration policy per
//! `feedback_single_cut_greenfield_migrations.md` precludes a
//! transitional default).

use serde::Deserialize;

/// Parsed `[dataplane]` config section. Both interface names are
/// required per architecture.md § 5.1; the parser refuses any TOML
/// shape that omits either field via `serde`'s default field
/// requirement.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataplaneConfig {
    /// Network interface the XDP `xdp_service_map_lookup` program
    /// attaches to for ingress traffic from clients (architecture.md
    /// § 5.1).
    pub client_iface: String,
    /// Network interface the XDP `xdp_reverse_nat_lookup` program
    /// attaches to for traffic returning from backend workloads
    /// (architecture.md § 5.1).
    pub backend_iface: String,
}

impl DataplaneConfig {
    /// Test-fixture helper: build a `DataplaneConfig` whose
    /// interface names point at the loopback. Used by every test
    /// `ServerConfig {}` construct site that does not exercise the
    /// real XDP attach path — production callers go through
    /// [`parse_dataplane_section`].
    ///
    /// On Linux (the canonical Lima test environment per
    /// `.claude/rules/testing.md` § "Running tests — Lima VM") the
    /// loopback interface is named `lo`; on macOS dev hosts (which
    /// run the tests inside Lima) the same name is reachable.
    #[must_use]
    pub fn loopback() -> Self {
        Self { client_iface: "lo".to_owned(), backend_iface: "lo".to_owned() }
    }
}

/// Top-level wrapper for deserialising the `[dataplane]` subtree.
/// Every other section in the input TOML is ignored — we are not the
/// authoritative parser for the rest of the control-plane config.
#[derive(Debug, Deserialize)]
struct TopLevel {
    #[serde(default)]
    dataplane: Option<DataplaneSection>,
}

/// Per-section deserialisation shape. `client_iface` and
/// `backend_iface` are required (serde rejects with a `missing field`
/// error when absent); `vip_allocator` is the
/// [`crate::vip_allocator_config`] subsection and is allowed to be
/// present without breaking this parser's `deny_unknown_fields` —
/// hence the explicit `vip_allocator` field even though this parser
/// does not consume it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataplaneSection {
    client_iface: String,
    backend_iface: String,
    #[serde(default)]
    #[allow(dead_code, reason = "co-located subsection parsed by `vip_allocator_config`")]
    vip_allocator: Option<toml::Value>,
}

/// Parse the `[dataplane]` section out of a TOML config string.
///
/// Returns:
///
/// - `Ok(DataplaneConfig { .. })` when the section is present and
///   both required fields are populated.
/// - `Err(ControlPlaneError::Validation { field: Some("dataplane"),
///   .. })` when the section is absent OR when a required field is
///   missing or the TOML is malformed.
///
/// # Errors
///
/// Returns [`crate::error::ControlPlaneError::Validation`] with
/// `field = Some("dataplane")` for every refusal shape — missing
/// section, missing required key, type mismatch, unknown field.
pub fn parse_dataplane_section(
    toml_input: &str,
) -> Result<DataplaneConfig, crate::error::ControlPlaneError> {
    let top: TopLevel =
        toml::from_str(toml_input).map_err(|err| crate::error::ControlPlaneError::Validation {
            message: format!("invalid [dataplane] section: {err}"),
            field: Some("dataplane".to_owned()),
        })?;

    let Some(section) = top.dataplane else {
        return Err(crate::error::ControlPlaneError::Validation {
            message: "missing required [dataplane] section in overdrive.toml \
                      (client_iface + backend_iface)"
                .to_owned(),
            field: Some("dataplane".to_owned()),
        });
    };

    Ok(DataplaneConfig { client_iface: section.client_iface, backend_iface: section.backend_iface })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use super::{DataplaneConfig, parse_dataplane_section};
    use crate::error::ControlPlaneError;

    /// S-BDB-12 unit closure: a TOML input with no `[dataplane]`
    /// section must produce a structured `Validation` error whose
    /// `field` names "dataplane" (so the operator's CLI / log
    /// surfacing can branch on the field without `Display`-grepping)
    /// and whose `message` names the two required keys verbatim.
    #[test]
    fn boot_refuses_when_dataplane_section_missing() {
        let result = parse_dataplane_section("");
        match result {
            Err(ControlPlaneError::Validation { message, field }) => {
                assert_eq!(field.as_deref(), Some("dataplane"));
                assert!(
                    message.contains("missing required [dataplane] section"),
                    "expected verbatim 'missing required [dataplane] section', got: {message}",
                );
                assert!(
                    message.contains("client_iface") && message.contains("backend_iface"),
                    "expected message to name both required keys, got: {message}",
                );
            }
            other => panic!("expected Validation {{ .. }} on missing section, got {other:?}"),
        }
    }

    #[test]
    fn parse_succeeds_when_both_required_fields_present() {
        let toml_input = r#"
[dataplane]
client_iface = "lb_veth_a"
backend_iface = "lb_veth_b"
"#;
        let cfg = parse_dataplane_section(toml_input).expect("valid section must parse");
        assert_eq!(
            cfg,
            DataplaneConfig {
                client_iface: "lb_veth_a".to_owned(),
                backend_iface: "lb_veth_b".to_owned(),
            }
        );
    }

    #[test]
    fn parse_rejects_when_client_iface_missing() {
        let toml_input = r#"
[dataplane]
backend_iface = "lb_veth_b"
"#;
        let result = parse_dataplane_section(toml_input);
        match result {
            Err(ControlPlaneError::Validation { field, .. }) => {
                assert_eq!(field.as_deref(), Some("dataplane"));
            }
            other => panic!("expected Validation on missing client_iface, got {other:?}"),
        }
    }

    #[test]
    fn parse_accepts_dataplane_vip_allocator_subsection_coexistence() {
        // The `[dataplane.vip_allocator]` subsection (owned by
        // `vip_allocator_config`) must not trip
        // `deny_unknown_fields` on the outer `[dataplane]` table —
        // they share the root namespace per architecture.md § 5.1.
        let toml_input = r#"
[dataplane]
client_iface = "lb_veth_a"
backend_iface = "lb_veth_b"

[dataplane.vip_allocator]
ranges = ["10.96.0.0/24"]
"#;
        let cfg = parse_dataplane_section(toml_input)
            .expect("co-located vip_allocator subsection must not break dataplane parser");
        assert_eq!(cfg.client_iface, "lb_veth_a");
    }
}
