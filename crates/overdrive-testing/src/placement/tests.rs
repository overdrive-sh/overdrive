//! The closed registry of guest groups and profiles, and the Metal-run
//! predicate, through their pinned surface: `GuestPlacement::declare` and the
//! pure core of `in_metal_run`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "acceptance tests assert their own preconditions and name protocol terms"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::{GuestPlacement, Sharing, metal_run_context_present};

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

/// Guards against over-rejection: every named environment stays declarable
/// (the rejection tests below are the pending ones).
/// CONTRACT_SHAPE: pure-function (a declaration is a value in, a value out).
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn the_registry_accepts_exactly_the_twenty_one_named_environments() {
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

/// The profile may be omitted: every group has a base profile.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn an_omitted_profile_selects_each_groups_base_profile() {
    for (group, _) in ENVIRONMENTS {
        assert!(
            panic_message(|| GuestPlacement::declare(group, "", Sharing::Isolated, "")).is_none(),
            "{group} has a base profile"
        );
    }
}

/// An unknown group is a build-time failure: at constant evaluation the
/// declaration panics with a fixed message.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test"]
fn an_unknown_group_is_rejected_by_name_at_declaration() {
    for group in ["cgroup-defualt", "", "CGROUP-DEFAULT", "host"] {
        let message = panic_message(|| GuestPlacement::declare(group, "", Sharing::Isolated, ""))
            .unwrap_or_else(|| panic!("group {group:?} was accepted"));
        assert!(message.contains("unknown guest group"), "{message}");
    }
}

/// An unknown profile, or a profile that belongs to another group, is rejected.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: guest-artifacts-declared-per-group"]
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

/// A Metal run context is present when any of the three variables is set and
/// non-empty, whatever its value: an `exclusive` class counts, so a guest-placed
/// body is never run in place on the shared host.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test"]
fn a_metal_run_is_present_when_any_one_of_the_three_variables_is_set_and_non_empty() {
    let cases: [(Option<&str>, Option<&str>, Option<&str>); 6] = [
        (Some("0123456789abcdef01234567"), None, None),
        (None, Some("ordinary"), None),
        (None, Some("exclusive"), None),
        (None, None, Some("/home/ubuntu/overdrive-worktrees/x")),
        (Some("x"), Some("ordinary"), Some("/y")),
        (Some(""), Some("exclusive"), Some("")),
    ];
    for (run_id, class, root) in cases {
        assert!(metal_run_context_present(run_id, class, root), "{run_id:?} {class:?} {root:?}");
    }
}

/// No variable, or only empty ones, is no Metal run.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test"]
fn no_metal_run_is_present_when_no_variable_is_set_or_every_set_one_is_empty() {
    let cases: [(Option<&str>, Option<&str>, Option<&str>); 4] = [
        (None, None, None),
        (Some(""), None, None),
        (Some(""), Some(""), Some("")),
        (None, Some(""), None),
    ];
    for (run_id, class, root) in cases {
        assert!(!metal_run_context_present(run_id, class, root), "{run_id:?} {class:?} {root:?}");
    }
}
