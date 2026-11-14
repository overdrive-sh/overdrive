//! PROBE V-11: the registry side. Single list of groups and profiles; a const fn
//! `declare` that panics (literal message) on an unknown group or profile.

#[derive(Debug, Clone, Copy)]
pub struct GuestPlacement {
    pub group: &'static str,
    pub profile: &'static str,
}

/// One list, consumed through a callback macro so a const and a message literal
/// both derive from it.
#[macro_export]
macro_rules! guest_groups {
    ($callback:ident) => {
        $crate::$callback!("probe", "core", "unload", "ulp", "mtls-relay", "cgroup-default", "stress", "nested-boundary")
    };
}

#[macro_export]
macro_rules! groups_as_const_slice {
    ($($name:literal),*) => { &[$($name),*] };
}

#[macro_export]
macro_rules! groups_as_text {
    ($($name:literal),*) => { concat!($($name, " "),*) };
}

/// The known set as a literal usable inside `concat!`.
#[macro_export]
macro_rules! known_guest_groups {
    () => { $crate::guest_groups!(groups_as_text) };
}

pub const GROUPS: &[&str] = guest_groups!(groups_as_const_slice);

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub const fn is_known_group(group: &str) -> bool {
    let mut i = 0;
    while i < GROUPS.len() {
        if str_eq(GROUPS[i], group) {
            return true;
        }
        i += 1;
    }
    false
}

impl GuestPlacement {
    /// Literal-only panics: the only form stable const evaluation accepts.
    pub const fn declare(group: &'static str, profile: &'static str) -> Self {
        if !is_known_group(group) {
            panic!("unknown guest group; the known groups are listed in overdrive_testing::placement::groups");
        }
        if !str_eq(profile, "base") && !str_eq(profile, "wide") {
            panic!("unknown guest profile for this group; the known profiles are listed in overdrive_testing::placement::groups");
        }
        Self { group, profile }
    }
}
