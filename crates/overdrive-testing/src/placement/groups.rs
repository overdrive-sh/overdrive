//! The closed registry of guest groups and profiles.
//!
//! Also the typed placement the attribute builds from it in constant context.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::must_use_candidate,
    dead_code,
    reason = "scaffold bodies are built by their delivery slice"
)]

/// The eight guest groups. Informative: the contract is the group name
/// strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestGroup {
    /// `probe`
    Probe,
    /// `core`
    Core,
    /// `unload`
    Unload,
    /// `ulp`
    Ulp,
    /// `mtls-relay`
    MtlsRelay,
    /// `cgroup-default`
    CgroupDefault,
    /// `stress`
    Stress,
    /// `nested-boundary`
    NestedBoundary,
}

/// The group names, the registry's single list.
const GROUP_NAMES: [&str; 8] =
    ["probe", "core", "unload", "ulp", "mtls-relay", "cgroup-default", "stress", "nested-boundary"];

/// True when `name` is one of the eight group names. Exact, case-sensitive.
///
/// The placement macros call it in constant context to name an unknown group in
/// the build error, so the macro crate holds no list of its own.
#[doc(hidden)]
pub const fn is_known_group(name: &str) -> bool {
    let wanted = name.as_bytes();
    let mut group = 0;
    while group < GROUP_NAMES.len() {
        let known = GROUP_NAMES[group].as_bytes();
        if bytes_equal(known, wanted) {
            return true;
        }
        group += 1;
    }
    false
}

const fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Whether a placement's guest is private to the test or shared by the tests
/// of one module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sharing {
    /// One guest per test.
    Isolated,
    /// One guest per declaring module.
    Module,
}

/// A guest placement, built in constant context by the attribute and by
/// `shared_guest!`.
#[derive(Debug, Clone, Copy)]
pub struct GuestPlacement {
    pub(super) group: &'static str,
    pub(super) profile: &'static str,
    pub(super) sharing: Sharing,
    pub(super) declaring_module: &'static str,
}

impl GuestPlacement {
    /// Declares a placement. Panics at constant evaluation on an unknown group
    /// (and, once the profiles are built, an unknown profile); `declaring_module` is the `module_path!()` of the module
    /// holding the declaration (empty for `Sharing::Isolated`).
    // SCAFFOLD: true for the profile: the group is validated against the closed
    // registry, the per-group profile check is built with the profiles.
    pub const fn declare(
        group: &'static str,
        profile: &'static str,
        sharing: Sharing,
        declaring_module: &'static str,
    ) -> Self {
        assert!(is_known_group(group), "unknown guest group");
        Self { group, profile, sharing, declaring_module }
    }
}

/// A lane placement: the in-guest fixture a lane test is driven by.
#[derive(Debug, Clone, Copy)]
pub struct LanePlacement {
    fixture: &'static str,
    owner: bool,
}

impl LanePlacement {
    /// The operator-side lane placement of `fixture`.
    pub const fn operator(fixture: &'static str) -> Self {
        Self { fixture, owner: false }
    }

    /// The owner-side lane placement of `fixture`.
    pub const fn owner(fixture: &'static str) -> Self {
        Self { fixture, owner: true }
    }
}
