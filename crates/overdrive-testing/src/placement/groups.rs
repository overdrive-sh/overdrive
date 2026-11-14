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

/// The closed registry: each group with its profiles. The first profile of a
/// group is its base profile, the one an omitted profile selects. Twenty-one
/// environments over eight groups.
const ENVIRONMENTS: [(&str, &[&str]); 8] = [
    ("probe", &["base"]),
    ("core", &["base"]),
    ("unload", &["base"]),
    ("ulp", &["base"]),
    ("mtls-relay", &["native", "relay"]),
    (
        "cgroup-default",
        &[
            "base",
            QUINT_PROFILE,
            "dummy-link",
            "host-route",
            "legb-tools",
            "legb-audit",
            "routed-route",
            "normal-gate",
            "nested",
            "nested-cohort",
            "nested-launch",
            "nested-launch-rustc",
            "nested-launch-lane",
        ],
    ),
    ("stress", &["base"]),
    ("nested-boundary", &["base"]),
];

/// The `cgroup-default` profile that stages the Quint runtime and the
/// `specs/quint` tree.
pub(crate) const QUINT_PROFILE: &str = "quint";

/// True when `name` is one of the eight group names. Exact, case-sensitive.
///
/// The placement macros call it in constant context to name an unknown group in
/// the build error, so the macro crate holds no list of its own.
#[doc(hidden)]
pub const fn is_known_group(name: &str) -> bool {
    group_index(name).is_some()
}

/// True when `profile` is one of `group`'s profiles, or empty (the omitted
/// profile, which selects the group's base profile). An unknown group has no
/// profiles. Exact, case-sensitive.
///
/// The placement macros call it in constant context to name an unknown profile
/// in the build error.
#[doc(hidden)]
pub const fn is_known_profile(group: &str, profile: &str) -> bool {
    let Some(group) = group_index(group) else { return false };
    if profile.is_empty() {
        return true;
    }
    let profiles = ENVIRONMENTS[group].1;
    let mut index = 0;
    while index < profiles.len() {
        if bytes_equal(profiles[index].as_bytes(), profile.as_bytes()) {
            return true;
        }
        index += 1;
    }
    false
}

/// The profile `profile` selects within `group`: the profile itself, or the
/// group's base profile when it is empty. `None` for an unknown group.
pub(crate) fn resolved_profile(group: &str, profile: &str) -> Option<&'static str> {
    let (_, profiles) = ENVIRONMENTS.iter().find(|(name, _)| *name == group)?;
    if profile.is_empty() {
        return profiles.first().copied();
    }
    profiles.iter().copied().find(|known| *known == profile)
}

const fn group_index(name: &str) -> Option<usize> {
    let mut group = 0;
    while group < ENVIRONMENTS.len() {
        if bytes_equal(ENVIRONMENTS[group].0.as_bytes(), name.as_bytes()) {
            return Some(group);
        }
        group += 1;
    }
    None
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
    /// The profile this placement selects: the declared one, or the group's
    /// base profile when none was declared.
    pub(crate) fn profile(&self) -> &'static str {
        resolved_profile(self.group, self.profile)
            .unwrap_or_else(|| unreachable!("`declare` admits only registry environments"))
    }

    /// Declares a placement. Panics at constant evaluation on an unknown group
    /// or on a profile that is not one of the group's (an empty profile selects
    /// the group's base profile); `declaring_module` is the `module_path!()` of
    /// the module holding the declaration (empty for `Sharing::Isolated`).
    pub const fn declare(
        group: &'static str,
        profile: &'static str,
        sharing: Sharing,
        declaring_module: &'static str,
    ) -> Self {
        assert!(is_known_group(group), "unknown guest group");
        assert!(is_known_profile(group, profile), "unknown guest profile for this group");
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

    /// The in-guest fixture this placement names.
    pub(crate) const fn fixture(&self) -> &'static str {
        self.fixture
    }

    /// Whether this is the owner-side placement (the host runs the fixture's
    /// script against the cell) rather than the operator-side one.
    pub(crate) const fn is_owner(&self) -> bool {
        self.owner
    }
}
