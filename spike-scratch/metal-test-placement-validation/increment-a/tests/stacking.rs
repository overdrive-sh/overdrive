//! PROBE V-2: does the placement attribute stack over the test attributes the
//! workspace uses? Each module declares its group once (`shared_guest!`) or
//! the test names it explicitly (`#[overdrive_test(guest = "...")]`).
//!
//! The probe macro has no guest hand-off: V-2 is about attribute stacking and
//! the records, not about booting guests.
use probe_macros::{overdrive_test, shared_guest};

/// Names the full test list this binary is expected to contain, so the last
/// test can compare the ELF records with it.
const EXPECTED: &[&str] = &[
    "tokio_stack::tokio_first_then_ours",
    "tokio_stack::ours_then_tokio_default",
    "tokio_stack::multi_thread_flavor",
    "tokio_stack::ignore_and_should_panic",
    "tokio_stack::should_panic_expected",
    "serial_stack::serial_then_tokio",
    "serial_stack::ours_then_serial",
    "proptest_stack::p_ours_only",
    "proptest_stack::p_test_then_ours",
    "proptest_stack::p_with_config",
    "proptest_stack::p_closure_form_in_fn",
    "quint_stack::quint_test_then_ours",
    "quint_stack::quint_run_then_ours",
    "trybuild_stack::trybuild_pass_fixture",
    "explicit_stack::explicit_form_plain",
    "explicit_stack::explicit_over_tokio",
    "records::section_matches_the_test_list",
];

mod tokio_stack {
    use super::*;
    shared_guest!(guest = "cgroup-default");

    #[tokio::test]
    #[overdrive_test]
    async fn tokio_first_then_ours() {
        tokio::task::yield_now().await;
    }

    #[overdrive_test]
    #[tokio::test]
    async fn ours_then_tokio_default() {
        tokio::task::yield_now().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[overdrive_test]
    async fn multi_thread_flavor() {
        tokio::task::yield_now().await;
    }

    // Reported by libtest as `ignored`; must still produce a record.
    #[test]
    #[ignore = "probe: an ignored test still carries a record"]
    #[overdrive_test]
    fn ignore_and_should_panic() {}

    #[test]
    #[should_panic(expected = "deliberate")]
    #[overdrive_test]
    fn should_panic_expected() {
        panic!("deliberate");
    }
}

mod serial_stack {
    use super::*;
    use serial_test::serial;
    shared_guest!(guest = "cgroup-default");

    #[tokio::test]
    #[serial(env)]
    #[overdrive_test]
    async fn serial_then_tokio() {}

    #[overdrive_test]
    #[serial(env)]
    #[test]
    fn ours_then_serial() {}
}

mod proptest_stack {
    use super::*;
    use proptest::prelude::*;
    shared_guest!(guest = "cgroup-default");

    proptest! {
        // The attribute inside the block adds `#[test]` to proptest's generated fn.
        #[overdrive_test]
        fn p_ours_only(x in 0_u8..10) {
            prop_assert!(x < 10);
        }

        #[test]
        #[overdrive_test]
        fn p_test_then_ours(x in 0_u8..10) {
            prop_assert!(x < 10);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(8))]
        #[overdrive_test]
        fn p_with_config(x in 0_u8..10, y in 0_u8..10) {
            prop_assert!(u16::from(x) + u16::from(y) < 20);
        }
    }

    // The closure form lives inside an ordinary fn: a plain attribute target.
    #[overdrive_test]
    fn p_closure_form_in_fn() {
        proptest!(|(x in 0_u8..10)| { prop_assert!(x < 10); });
    }
}

mod quint_stack {
    use super::*;
    use anyhow::Result as AnyResult;
    use quint_connect::{Config, Driver, Step, quint_run, quint_test, switch};
    use serde::Deserialize;
    shared_guest!(guest = "cgroup-default");

    #[derive(Default)]
    struct D {
        x: i64,
    }

    #[derive(Eq, PartialEq, Deserialize, Debug)]
    struct S {
        x: i64,
    }

    impl quint_connect::State<D> for S {
        fn from_driver(driver: &D) -> AnyResult<Self> {
            Ok(Self { x: driver.x })
        }
    }

    // quint_test drives named runs, which carry no `mbt::actionTaken`, so the
    // spec records `conf_step` and the driver reads the action from it.
    #[derive(Default)]
    struct DT(D);
    impl quint_connect::State<DT> for S {
        fn from_driver(driver: &DT) -> AnyResult<Self> {
            Ok(Self { x: driver.0.x })
        }
    }
    impl Driver for DT {
        type State = S;
        fn config() -> Config {
            Config { state: &[], nondet: &["conf_step"] }
        }
        fn step(&mut self, step: &Step) -> AnyResult<()> {
            switch!(step {
                ev_init => self.0.x = 0,
                ev_step => self.0.x += 1,
            })
        }
    }

    // quint_run uses Quint's builtin `mbt::actionTaken` (action names).
    impl Driver for D {
        type State = S;
        fn step(&mut self, step: &Step) -> AnyResult<()> {
            switch!(step {
                init => self.x = 0,
                step => self.x += 1,
            })
        }
    }

    // quint_test OUTSIDE (listed first): it emits `#[test]`, re-emits the
    // attributes below it, and replaces the fn (and its `impl Driver` return)
    // with a `()` fn; ours then sees a `()` fn that already has `#[test]`.
    #[quint_test(spec = "spec/probe.qnt", main = "probe", test = "counts", max_samples = 1)]
    #[overdrive_test]
    fn quint_test_then_ours() -> impl Driver {
        DT::default()
    }

    #[quint_run(spec = "spec/probe.qnt", main = "probe", max_samples = 2, max_steps = 3)]
    #[overdrive_test]
    fn quint_run_then_ours() -> impl Driver {
        D::default()
    }
}

mod trybuild_stack {
    use super::*;
    shared_guest!(guest = "cgroup-default");

    #[overdrive_test]
    fn trybuild_pass_fixture() {
        let t = trybuild::TestCases::new();
        t.pass("tests/ui/pass_fixture.rs");
    }
}

mod explicit_stack {
    use super::*;
    // No module declaration here: the explicit form names its own group.
    #[overdrive_test(guest = "core")]
    fn explicit_form_plain() {}

    #[overdrive_test(guest = "core")]
    #[tokio::test]
    async fn explicit_over_tokio() {}
}

mod records {
    use super::*;
    #[overdrive_test(guest = "core")]
    fn section_matches_the_test_list() {
        let records = placement_validation_probe::placement_records();
        placement_validation_probe::dump("stacking");
        let mut names: Vec<String> = records
            .iter()
            .map(|(n, _)| n.strip_prefix("stacking::").unwrap_or(n).to_string())
            .collect();
        names.sort();
        let mut expected: Vec<String> = super::EXPECTED.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(names, expected, "one record per test, none duplicated, none missing");
    }
}
