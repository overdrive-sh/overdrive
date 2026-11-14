//! The closed registry of guest groups and profiles, and the Metal-run
//! predicate, through their pinned surface: `GuestPlacement::declare` and
//! `in_metal_run()`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "acceptance tests assert their own preconditions and name protocol terms"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use overdrive_testing_macros::placement;
use serial_test::serial;

use super::{GuestPlacement, Sharing, in_metal_run};

placement!(host_pure);

/// The 21 environments over 8 groups the registry is closed over: each group's
/// profiles, the base profile spelled as the empty string.
const ENVIRONMENTS: &[(&str, &[&str])] = &[
    ("probe", &["base"]),
    ("core", &["base"]),
    ("unload", &["base"]),
    ("ulp", &["base"]),
    ("stress", &["base"]),
    ("nested-boundary", &["base"]),
    ("mtls-relay", &["native", "relay"]),
    (
        "cgroup-default",
        &[
            "base",
            "quint",
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
];

/// Built in constant context, as the attribute and `shared_guest!` build it.
const A_PLACEMENT_BUILT_AT_COMPILE_TIME: GuestPlacement =
    GuestPlacement::declare("cgroup-default", "base", Sharing::Module, "a::module");

fn panic_message(declare: impl FnOnce() -> GuestPlacement) -> Option<String> {
    let outcome = catch_unwind(AssertUnwindSafe(declare));
    let payload = outcome.err()?;
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .or_else(|| Some(String::new()))
}

/// A regression guard, not a RED test: the scaffold's `declare` stores what it
/// is given, so this passes today. It stays true once the registry validates:
/// every named environment is declarable (the rejection tests below are the RED
/// ones). Activated with the registry. Placement on activation:
/// `placement!(host_pure)` on the module (observes nothing outside its process).
/// CONTRACT_SHAPE: pure-function (a declaration is a value in, a value out).
#[test]
fn the_registry_accepts_each_named_environment() {
    let mut accepted = 0;
    for (group, profiles) in ENVIRONMENTS {
        for profile in *profiles {
            for sharing in [Sharing::Isolated, Sharing::Module] {
                let module = if matches!(sharing, Sharing::Module) { "a::module" } else { "" };
                assert!(
                    panic_message(|| GuestPlacement::declare(group, profile, sharing, module))
                        .is_none(),
                    "({group}, {profile}) is a named environment"
                );
            }
            accepted += 1;
        }
    }
    assert_eq!(accepted, 21, "twenty-one environments over eight groups");
    let _ = A_PLACEMENT_BUILT_AT_COMPILE_TIME;
}

/// The profile may be omitted: every group has a base profile. A regression
/// guard like the test above (the scaffold accepts it today). Placement on
/// activation: `placement!(host_pure)` on the module.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_omitted_profile_selects_each_groups_base_profile() {
    for (group, _) in ENVIRONMENTS {
        assert!(
            panic_message(|| GuestPlacement::declare(group, "", Sharing::Isolated, "")).is_none(),
            "{group} has a base profile"
        );
    }
}

/// An unknown group is a build-time failure: at constant evaluation the
/// declaration panics with a fixed message. The behaviour is built by the
/// guest-placement slice; the test needs the host-pure form to be placed on a
/// Metal run, so it activates with that form. Placement on activation:
/// `placement!(host_pure)` on the module.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_unknown_group_is_rejected_by_name_at_declaration() {
    for group in ["cgroup-defualt", "", "CGROUP-DEFAULT", "host"] {
        let message = panic_message(|| GuestPlacement::declare(group, "", Sharing::Isolated, ""))
            .unwrap_or_else(|| panic!("group {group:?} was accepted"));
        assert!(message.contains("unknown guest group"), "{message}");
    }
}

/// An unknown profile, or a profile that belongs to another group, is rejected.
/// Placement on activation: `placement!(host_pure)` on the module.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_unknown_profile_or_one_of_another_group_is_rejected() {
    for (group, profile) in [
        ("core", "quint"),
        ("probe", "native"),
        ("mtls-relay", "base"),
        ("cgroup-default", "nested-cohort-xl"),
        ("cgroup-default", "native"),
    ] {
        let message =
            panic_message(|| GuestPlacement::declare(group, profile, Sharing::Isolated, ""))
                .unwrap_or_else(|| panic!("({group}, {profile}) was accepted"));
        assert!(message.contains("unknown guest profile"), "{message}");
    }
}

// ----- the Metal-run predicate ----------------------------------------------

const METAL_VARIABLES: [&str; 3] =
    ["OVERDRIVE_METAL_RUN_ID", "OVERDRIVE_METAL_EXECUTION_CLASS", "OVERDRIVE_METAL_SOURCE_ROOT"];

/// Sets the three Metal variables for the length of a test and restores the
/// prior values on drop, panic included. Every test that holds one is
/// `#[serial(env)]`.
struct MetalEnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl MetalEnvGuard {
    fn scoped(values: [Option<&str>; 3]) -> Self {
        let mut saved = Vec::with_capacity(METAL_VARIABLES.len());
        for (name, value) in METAL_VARIABLES.into_iter().zip(values) {
            saved.push((name, std::env::var_os(name)));
            // SAFETY: `#[serial(env)]` gives this test exclusive access to the
            // process environment for its whole duration.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        Self { saved }
    }
}

impl Drop for MetalEnvGuard {
    fn drop(&mut self) {
        for (name, prior) in self.saved.drain(..) {
            // SAFETY: as in `scoped`; the test that holds the guard is still
            // `#[serial(env)]` while it drops.
            unsafe {
                match prior {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// A Metal run context is present when any of the three variables is set and
/// non-empty, whatever its value: an `exclusive` class counts, so a guest-placed
/// body is never run in place on the shared host.
/// `in_metal_run()` is built by the guest-placement slice; the test needs the
/// host-pure form to be placed on a Metal run, so it activates with that form.
/// Placement on activation: `placement!(host_pure)` on the module.
/// CONTRACT_SHAPE: pure-function (the process environment in, a verdict out).
#[test]
#[serial(env)]
fn a_metal_run_is_present_when_any_one_of_the_three_variables_is_set_and_non_empty() {
    let cases: [[Option<&str>; 3]; 6] = [
        [Some("0123456789abcdef01234567"), None, None],
        [None, Some("ordinary"), None],
        [None, Some("exclusive"), None],
        [None, None, Some("/home/ubuntu/overdrive-worktrees/x")],
        [Some("x"), Some("ordinary"), Some("/y")],
        [Some(""), Some("exclusive"), Some("")],
    ];
    for values in cases {
        let _environment = MetalEnvGuard::scoped(values);
        assert!(in_metal_run(), "{values:?}");
    }
}

/// No variable, or only empty ones, is no Metal run. Activates with the
/// host-pure form, like the test above.
/// Placement on activation: `placement!(host_pure)` on the module.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[serial(env)]
fn no_metal_run_is_present_when_no_variable_is_set_or_every_set_one_is_empty() {
    let cases: [[Option<&str>; 3]; 4] = [
        [None, None, None],
        [Some(""), None, None],
        [Some(""), Some(""), Some("")],
        [None, Some(""), None],
    ];
    for values in cases {
        let _environment = MetalEnvGuard::scoped(values);
        assert!(!in_metal_run(), "{values:?}");
    }
}
