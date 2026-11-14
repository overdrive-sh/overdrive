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
    group: &'static str,
    profile: &'static str,
    sharing: Sharing,
    declaring_module: &'static str,
}

impl GuestPlacement {
    /// Declares a placement. Panics at constant evaluation on an unknown group
    /// or profile; `declaring_module` is the `module_path!()` of the module
    /// holding the declaration (empty for `Sharing::Isolated`).
    // SCAFFOLD: true. Stores its arguments; the closed-registry validation is
    // built with the registry.
    pub const fn declare(
        group: &'static str,
        profile: &'static str,
        sharing: Sharing,
        declaring_module: &'static str,
    ) -> Self {
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
