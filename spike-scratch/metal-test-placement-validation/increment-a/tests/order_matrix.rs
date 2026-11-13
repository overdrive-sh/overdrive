//! PROBE V-2 (second part): the order-insensitive `record_only` form. Every
//! test below is registered with libtest exactly once, whichever side of the
//! test attribute the placement attribute is written on, and whatever the
//! test's return type. The record is a sibling static, the fn is untouched.
use probe_macros::{overdrive_test, shared_guest};

const EXPECTED: &[&str] = &[
    "matrix::ours_outer_plain_test",
    "matrix::ours_inner_plain_test",
    "matrix::ours_inner_should_panic",
    "matrix::ours_inner_tokio",
    "matrix::ours_outer_tokio",
    "matrix::ours_inner_returns_result",
    "matrix::ours_outer_returns_result",
    "matrix::quint_outer_ours_inner",
    "matrix::ours_outer_quint_inner",
    "matrix::quint_run_outer_ours_inner",
    "matrix::ours_outer_quint_run_inner",
    "matrix::p_test_then_ours",
    "matrix::p_ours_then_test",
    "matrix::explicit_group_record_only",
    "matrix::section_matches_the_test_list",
];

mod matrix {
    use super::*;
    use anyhow::Result as AnyResult;
    use proptest::prelude::*;
    use quint_connect::{Config, Driver, Step, quint_run, quint_test, switch};
    use serde::Deserialize;
    shared_guest!(guest = "cgroup-default");

    #[overdrive_test(record_only)]
    #[test]
    fn ours_outer_plain_test() {}

    #[test]
    #[overdrive_test(record_only)]
    fn ours_inner_plain_test() {}

    #[test]
    #[should_panic(expected = "deliberate")]
    #[overdrive_test(record_only)]
    fn ours_inner_should_panic() {
        panic!("deliberate");
    }

    #[tokio::test]
    #[overdrive_test(record_only)]
    async fn ours_inner_tokio() {}

    #[overdrive_test(record_only)]
    #[tokio::test]
    async fn ours_outer_tokio() {}

    #[test]
    #[overdrive_test(record_only)]
    fn ours_inner_returns_result() -> Result<(), String> {
        Ok(())
    }

    #[overdrive_test(record_only)]
    #[test]
    fn ours_outer_returns_result() -> Result<(), String> {
        Ok(())
    }

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
    #[overdrive_test(record_only)]
    fn quint_outer_ours_inner() -> impl Driver {
        DT::default()
    }

    // The placement attribute outermost: it sees the original `impl Driver` fn
    // BEFORE quint_test rewrites it.
    #[overdrive_test(record_only)]
    #[quint_test(spec = "spec/probe.qnt", main = "probe", test = "counts", max_samples = 1)]
    fn ours_outer_quint_inner() -> impl Driver {
        DT::default()
    }

    #[quint_run(spec = "spec/probe.qnt", main = "probe", max_samples = 2, max_steps = 3)]
    #[overdrive_test(record_only)]
    fn quint_run_outer_ours_inner() -> impl Driver {
        D::default()
    }

    #[overdrive_test(record_only)]
    #[quint_run(spec = "spec/probe.qnt", main = "probe", max_samples = 2, max_steps = 3)]
    fn ours_outer_quint_run_inner() -> impl Driver {
        D::default()
    }

    proptest! {
        #[test]
        #[overdrive_test(record_only)]
        fn p_test_then_ours(x in 0_u8..10) {
            prop_assert!(x < 10);
        }

        #[overdrive_test(record_only)]
        #[test]
        fn p_ours_then_test(x in 0_u8..10) {
            prop_assert!(x < 10);
        }
    }

    #[overdrive_test(guest = "core", record_only)]
    #[test]
    fn explicit_group_record_only() {}

    #[overdrive_test(guest = "core", record_only)]
    #[test]
    fn section_matches_the_test_list() {
        let records = placement_validation_probe::placement_records();
        placement_validation_probe::dump("order_matrix");
        let mut names: Vec<String> =
            records.iter().map(|(n, _)| n.strip_prefix("order_matrix::").unwrap_or(n).to_string()).collect();
        names.sort();
        let mut expected: Vec<String> = super::EXPECTED.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(names, expected, "one record per test");
    }
}
