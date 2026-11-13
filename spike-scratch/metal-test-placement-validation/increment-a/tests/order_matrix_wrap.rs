//! PROBE V-2 (third part): the body-wrapping form that never adds `#[test]`
//! (`strict`). The gate (record + the early `return;` of the real guest
//! hand-off) wraps the body; every test is registered with libtest once,
//! whichever side of the test attribute the placement attribute is on.
use probe_macros::{overdrive_test, shared_guest};

const EXPECTED: &[&str] = &[
    "wrapmatrix::ours_outer_plain_test",
    "wrapmatrix::ours_inner_plain_test",
    "wrapmatrix::ours_inner_should_panic",
    "wrapmatrix::ours_inner_tokio",
    "wrapmatrix::ours_outer_tokio",
    "wrapmatrix::quint_outer_ours_inner",
    "wrapmatrix::quint_run_outer_ours_inner",
    "wrapmatrix::p_test_then_ours",
    "wrapmatrix::p_ours_then_test",
    "wrapmatrix::explicit_group_strict",
    "wrapmatrix::section_matches_the_test_list",
];

mod wrapmatrix {
    use super::*;
    use anyhow::Result as AnyResult;
    use proptest::prelude::*;
    use quint_connect::{Config, Driver, Step, quint_run, quint_test, switch};
    use serde::Deserialize;
    shared_guest!(guest = "cgroup-default");

    #[overdrive_test(strict)]
    #[test]
    fn ours_outer_plain_test() {}

    #[test]
    #[overdrive_test(strict)]
    fn ours_inner_plain_test() {}

    #[test]
    #[should_panic(expected = "deliberate")]
    #[overdrive_test(strict)]
    fn ours_inner_should_panic() {
        panic!("deliberate");
    }

    #[tokio::test]
    #[overdrive_test(strict)]
    async fn ours_inner_tokio() {}

    #[overdrive_test(strict)]
    #[tokio::test]
    async fn ours_outer_tokio() {}

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

    #[quint_test(spec = "spec/probe.qnt", main = "probe", test = "counts", max_samples = 1)]
    #[overdrive_test(strict)]
    fn quint_outer_ours_inner() -> impl Driver {
        DT::default()
    }

    #[quint_run(spec = "spec/probe.qnt", main = "probe", max_samples = 2, max_steps = 3)]
    #[overdrive_test(strict)]
    fn quint_run_outer_ours_inner() -> impl Driver {
        D::default()
    }

    proptest! {
        #[test]
        #[overdrive_test(strict)]
        fn p_test_then_ours(x in 0_u8..10) {
            prop_assert!(x < 10);
        }

        #[overdrive_test(strict)]
        #[test]
        fn p_ours_then_test(x in 0_u8..10) {
            prop_assert!(x < 10);
        }
    }

    #[overdrive_test(guest = "core", strict)]
    #[test]
    fn explicit_group_strict() {}

    #[overdrive_test(guest = "core", strict)]
    #[test]
    fn section_matches_the_test_list() {
        let records = placement_validation_probe::placement_records();
        placement_validation_probe::dump("order_matrix_wrap");
        let mut names: Vec<String> =
            records.iter().map(|(n, _)| n.strip_prefix("order_matrix_wrap::").unwrap_or(n).to_string()).collect();
        names.sort();
        let mut expected: Vec<String> = super::EXPECTED.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(names, expected, "one record per test");
    }
}
