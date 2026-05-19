//! `[dataplane.vip_allocator]` TOML parser surface.
//!
//! Step 02-02 of `service-vip-allocator`, amended by step 02-03c
//! (ADR-0049 § Amendments → 2026-05-15, Alt-E accepted). Owns the
//! boot-time *parser* for the operator-supplied VIP-allocator config:
//!
//! ```toml
//! [dataplane.vip_allocator]
//! ranges   = ["10.96.0.0/24"]              # required when section present
//! reserved = ["10.96.0.1", "10.96.0.254"]  # optional, defaults to []
//! ```
//!
//! Type-level invariants (overlapping CIDRs, reserved-outside-range,
//! zero capacity) are NOT checked here — they delegate to
//! [`VipRange::new`] in `overdrive-dataplane`. This module owns only:
//!
//! 1. Section presence detection — `Ok(None)` when
//!    `[dataplane.vip_allocator]` is absent so the caller can
//!    substitute [`VipRange::default()`] per ADR-0049's
//!    default-with-override posture. Missing is no longer an error.
//! 2. TOML deserialisation of `ranges` + `reserved` into the
//!    constructor's input shape.
//! 3. Delegation to [`VipRange::new`] for the three type-level
//!    invariants.
//! 4. Emission of the structured `health.startup.refused` tracing
//!    event on every refusal — operator-visible at boot, structured so
//!    the §12 investigation agent can branch on the typed variant
//!    rather than `Display`-grep a string. Refusal applies only to
//!    malformed-present configs; absent sections proceed silently to
//!    the default.
//!
//! Per ADR-0049 § 5b as amended 2026-05-15. The three invalid TOML
//! shapes that still surface here as parser-surface errors are
//! exercised at the type level by [`VipRange::new`]'s own unit tests
//! in step 01-01.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use overdrive_dataplane::allocators::{VipAllocatorConfigError, VipRange};
use serde::Deserialize;
use thiserror::Error;

/// Dotted-path name of the required config section. Single source of
/// truth: every diagnostic and the [`VipAllocatorConfigError::Missing`]
/// constructor read from this constant so the operator-facing message
/// and the variant payload cannot drift.
pub const VIP_ALLOCATOR_SECTION: &str = "dataplane.vip_allocator";

/// Boot-time failures from the VIP-allocator config parser.
///
/// Wraps [`VipAllocatorConfigError`] via `#[from]` so the
/// composition-root error chain stays structurally typed
/// (`matches!(e, ControlPlaneError::VipAllocatorConfig(_))` reaches a
/// real variant, not a stringified `Internal`). The second variant
/// covers TOML parse failures (malformed input, wrong types) — distinct
/// from the type-level invariant failures so the operator gets the
/// right "fix your CIDR string" vs "you have an overlapping range"
/// remediation, per `.claude/rules/development.md` § "Distinct failure
/// modes get distinct error variants".
///
/// **Absent sections are NOT an error** per ADR-0049 § Amendments →
/// 2026-05-15. [`parse_vip_allocator_section`] returns `Ok(None)` when
/// `[dataplane.vip_allocator]` is missing; only malformed-present
/// configs flow through this enum.
#[derive(Debug, Error)]
pub enum VipAllocatorBootError {
    /// The TOML parsed successfully and the section is present, but
    /// `VipRange::new` rejected the (ranges, reserved) inputs —
    /// overlapping CIDRs, a reserved address outside every configured
    /// range, or zero effective capacity. The three `Display` shapes
    /// name the offending input verbatim so the operator can edit
    /// the config without re-checking diagnostics.
    #[error(transparent)]
    Config(#[from] VipAllocatorConfigError),

    /// The TOML input is malformed (unparseable, wrong types, unknown
    /// fields). Distinct from [`Self::Config`] because the operator's
    /// fix differs: a `TomlParse` is "your `ranges` value is not a
    /// list of strings"; a `Config` variant is "your CIDR overlaps
    /// with another".
    #[error("invalid [{section}] config: {source}")]
    TomlParse {
        /// Section the parser was reading.
        section: &'static str,
        /// Underlying TOML decode error.
        #[source]
        source: toml::de::Error,
    },
}

/// Result alias for boot-time parser callers.
pub type Result<T, E = VipAllocatorBootError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------
// Internal TOML shapes — deserialised, then thrown away. The persisted
// value (per `.claude/rules/development.md` § "Persist inputs, not
// derived state") is the operator's TOML file on disk; the `VipRange`
// is derived in-process from that input every boot.
// ---------------------------------------------------------------------------

/// Top-level wrapper for deserialising just the `[dataplane]` /
/// `[dataplane.vip_allocator]` subtree out of an arbitrary TOML
/// document. Every other section is ignored — we are not the
/// authoritative parser for the rest of the control-plane config (no
/// such parser exists today; the current boot path uses CLI flags).
#[derive(Debug, Deserialize)]
struct TopLevel {
    #[serde(default)]
    dataplane: Option<DataplaneSection>,
}

#[derive(Debug, Deserialize)]
struct DataplaneSection {
    #[serde(default)]
    vip_allocator: Option<VipAllocatorSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VipAllocatorSection {
    /// Required: one or more IPv4 CIDR ranges.
    ranges: Vec<Ipv4Net>,
    /// Optional: reserved addresses excluded from allocation. Defaults
    /// to empty.
    #[serde(default)]
    reserved: Vec<Ipv4Addr>,
}

// ---------------------------------------------------------------------------
// Public parser surface.
// ---------------------------------------------------------------------------

/// Parse the `[dataplane.vip_allocator]` section out of a TOML config
/// string and validate it via [`VipRange::new`].
///
/// Returns:
///
/// - `Ok(None)` when the section is absent — the caller substitutes
///   [`VipRange::default()`] per ADR-0049's default-with-override
///   posture (amendment 2026-05-15). No diagnostic event is emitted
///   on this path; absent is a normal-operations shape.
/// - `Ok(Some(range))` when the section is present and validates.
/// - `Err(_)` when the section is present but malformed (TOML parse
///   error) or invalid (overlapping CIDRs, reserved-outside-range,
///   zero capacity).
///
/// On refusal, emits a structured `tracing::error!` event named
/// `health.startup.refused` (target `overdrive::health`) carrying the
/// typed error variant as `cause = %err`. The event fires BEFORE the
/// `Err(_)` returns so a caller that drops the `Err` still leaves the
/// operator-facing trail on stderr.
///
/// # Errors
///
/// - [`VipAllocatorBootError::Config`] wrapping `OverlappingRanges`,
///   `ReservedOutsideRange`, or `ZeroCapacity` from `VipRange::new`.
/// - [`VipAllocatorBootError::TomlParse`] when the TOML is malformed
///   or carries a value with the wrong type.
pub fn parse_vip_allocator_section(toml_input: &str) -> Result<Option<VipRange>> {
    let result = parse_inner(toml_input);
    if let Err(ref err) = result {
        emit_startup_refused(err);
    }
    result
}

fn parse_inner(toml_input: &str) -> Result<Option<VipRange>> {
    let top: TopLevel = toml::from_str(toml_input).map_err(|source| {
        VipAllocatorBootError::TomlParse { section: VIP_ALLOCATOR_SECTION, source }
    })?;

    let Some(section) = top.dataplane.and_then(|d| d.vip_allocator) else {
        // Absent section is no longer an error per ADR-0049 amendment
        // 2026-05-15 — caller substitutes VipRange::default().
        return Ok(None);
    };

    let reserved: BTreeSet<Ipv4Addr> = section.reserved.into_iter().collect();
    let range = VipRange::new(section.ranges, reserved)?;
    Ok(Some(range))
}

fn emit_startup_refused(err: &VipAllocatorBootError) {
    tracing::error!(
        target: "overdrive::health",
        event = "health.startup.refused",
        cause = %err,
        "VIP allocator config refused; control-plane will not start"
    );
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    //! In-crate unit smoke tests for the parser. Acceptance scenarios
    //! live at `tests/acceptance/vip_allocator_config_parsing.rs` —
    //! these here are colocated tautology checks that fire if the
    //! module-internal shape changes (e.g., a TOML field rename) and
    //! the acceptance tests' coarser assertions miss it.

    use super::{VipAllocatorBootError, parse_vip_allocator_section};

    /// ADR-0049 amendment 2026-05-15 (Alt-E): the parser now treats a
    /// missing `[dataplane.vip_allocator]` section as a non-error;
    /// the caller substitutes `VipRange::default()`. Surface contract:
    /// `Ok(None)` rather than `Err(Config(Missing))`. This unit test
    /// pins the contract so a regression to "refuse on missing" fails
    /// loud at PR time.
    #[test]
    fn parse_vip_allocator_section_returns_none_on_missing() {
        // Empty TOML: no [dataplane] at all → no section.
        let result = parse_vip_allocator_section("");
        match result {
            Ok(None) => {}
            other => {
                panic!("expected Ok(None) on missing section, got {other:?}");
            }
        }

        // [dataplane] present but [dataplane.vip_allocator] absent →
        // still Ok(None). This was the pre-amendment "Missing" path.
        let toml_with_dataplane = "[dataplane]\n";
        let result = parse_vip_allocator_section(toml_with_dataplane);
        match result {
            Ok(None) => {}
            other => {
                panic!("expected Ok(None) when only [dataplane] is present, got {other:?}");
            }
        }
    }

    #[test]
    fn unknown_field_under_section_rejected() {
        // `deny_unknown_fields` should fire on an unrecognised key.
        let toml_str = r#"
[dataplane.vip_allocator]
ranges = ["10.96.0.0/24"]
unexpected_field = true
"#;
        match parse_vip_allocator_section(toml_str) {
            Err(VipAllocatorBootError::TomlParse { .. }) => {}
            other => panic!("expected TomlParse on unknown field, got {other:?}"),
        }
    }
}
