// NEGATIVE: the placement attribute listed ABOVE quint_test sees the original
// `impl Driver` return type before quint_test has rewritten the fn.
use probe_macros::{overdrive_test, shared_guest};
use quint_connect::{Config, Driver, Step, quint_test};

shared_guest!(guest = "cgroup-default");

#[derive(Default)]
struct D;
#[derive(Eq, PartialEq, serde::Deserialize, Debug)]
struct S;
impl quint_connect::State<D> for S {
    fn from_driver(_: &D) -> anyhow::Result<Self> { Ok(S) }
}
impl Driver for D {
    type State = S;
    fn config() -> Config { Config { state: &[], nondet: &["conf_step"] } }
    fn step(&mut self, _: &Step) -> anyhow::Result<()> { Ok(()) }
}

#[overdrive_test]
#[quint_test(spec = "spec/probe.qnt", main = "probe", test = "counts", max_samples = 1)]
fn quint_above() -> impl Driver { D }
