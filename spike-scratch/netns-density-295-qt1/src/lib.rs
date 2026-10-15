//! Spike QT1 (netns-density-295, D-UP-1). Throwaway.
//!
//! `Bank` is the "owner": a tiny Rust state machine. The quint-connect drivers
//! below replay `spec/bank.qnt` against it.

/// The owner under test.
#[derive(Debug, Default)]
pub struct Bank {
    a: i64,
    b: i64,
}

impl Bank {
    pub fn init(&mut self) {
        self.a = 0;
        self.b = 0;
    }

    fn slot(&mut self, acct: &str) -> &mut i64 {
        match acct {
            "a" => &mut self.a,
            "b" => &mut self.b,
            other => panic!("unknown account {other}"),
        }
    }

    pub fn deposit(&mut self, acct: &str, amt: i64) {
        *self.slot(acct) += amt;
    }

    pub fn withdraw(&mut self, acct: &str, amt: i64) {
        // `broken_owner`: subtract one too few — the comparison must catch it.
        let taken = if cfg!(feature = "broken_owner") { amt - 1 } else { amt };
        *self.slot(acct) -= taken;
    }

    pub fn transfer(&mut self, amt: i64) {
        self.a -= amt;
        self.b += amt;
    }

    /// Pinned observation accessor the projection reads.
    pub fn balances(&self) -> (i64, i64) {
        (self.a, self.b)
    }
}

#[cfg(test)]
mod conformance {
    use super::Bank;
    use itf::Value;
    use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
    use serde::Deserialize;

    const SPEC: &str = "spec/bank.qnt";

    /// Projected state. ITF variable names are qualified (`<main>::<module>::a`),
    /// so `from_spec` matches on the unqualified suffix. (The derive only
    /// satisfies the trait bound; `from_spec` is the deserialisation used.)
    #[derive(Debug, PartialEq, Deserialize)]
    pub struct BankState {
        a: i64,
        b: i64,
    }

    fn var(rec: &itf::value::Record, name: &str) -> Result<i64> {
        let suffix = format!("::{name}");
        let v = rec
            .iter()
            .find(|(k, _)| k.as_str() == name || k.ends_with(&suffix))
            .map(|(_, v)| v.clone())
            .ok_or_else(|| anyhow::anyhow!("missing var {name}"))?;
        Ok(i64::deserialize(v)?)
    }

    fn project(value: Value) -> Result<BankState> {
        let Value::Record(rec) = value else { anyhow::bail!("state is not a record") };
        Ok(BankState { a: var(&rec, "a")?, b: var(&rec, "b")? })
    }

    // ---------------------------------------------------------------- one replay body
    fn replay(bank: &mut Bank, step: &Step) -> Result {
        switch!(step {
            Init => bank.init(),
            // `--mbt` (quint run) names the init state after the `init` action
            init => bank.init(),
            Deposit(acct: String, amt: i64) => bank.deposit(&acct, amt),
            Withdraw(acct: String, amt: i64) => bank.withdraw(&acct, amt),
            Transfer(amt: i64) => bank.transfer(amt),
        })
    }

    macro_rules! driver {
        ($name:ident, $nondet:expr) => {
            #[derive(Default)]
            pub struct $name {
                bank: Bank,
            }
            impl State<$name> for BankState {
                fn from_spec(value: Value) -> Result<Self> {
                    project(value)
                }
                fn from_driver(d: &$name) -> Result<Self> {
                    let (a, b) = d.bank.balances();
                    Ok(BankState { a, b })
                }
            }
            impl Driver for $name {
                type State = BankState;
                fn config() -> Config {
                    Config { state: &[], nondet: $nondet }
                }
                fn step(&mut self, step: &Step) -> Result {
                    replay(&mut self.bank, step)
                }
            }
        };
    }

    // RED control: default config (reads mbt::actionTaken / mbt::nondetPicks).
    driver!(MbtDriver, &[]);
    // GREEN: conf_step on the conformance main `bank_ok`.
    driver!(OkDriver, &["bank_ok::bank::conf_step"]);
    // GREEN: qualified-instance main `bank_as`.
    driver!(AsDriver, &["bank_as::inst::conf_step"]);
    // Path probes.
    driver!(BareDriver, &["conf_step"]);
    driver!(SegmentedDriver, &["bank_ok", "bank", "conf_step"]);
    // Single-instance probe: conformance driver pointed at the NON-recording main.
    driver!(McDriver, &["bank_mc::bank::conf_step"]);

    // ---------------------------------------------------------------- RED
    #[quint_test(spec = "spec/bank.qnt", main = "bank_ok", test = "DepositThenWithdraw")]
    fn red_mbt_named_run() -> impl Driver {
        MbtDriver::default()
    }

    // ---------------------------------------------------------------- GREEN
    #[quint_test(spec = "spec/bank.qnt", main = "bank_ok", test = "DepositThenWithdraw")]
    fn green_named_deposit_then_withdraw() -> impl Driver {
        OkDriver::default()
    }

    #[quint_test(spec = "spec/bank.qnt", main = "bank_ok", test = "DepositTransferDeposit")]
    fn green_named_deposit_transfer_deposit() -> impl Driver {
        OkDriver::default()
    }

    #[quint_run(spec = "spec/bank.qnt", main = "bank_ok", max_samples = 50, max_steps = 15)]
    fn green_run_random() -> impl Driver {
        OkDriver::default()
    }

    // #[quint_run] with the default (mbt) config, for comparison: --mbt is passed.
    #[quint_run(spec = "spec/bank.qnt", main = "bank_ok", max_samples = 50, max_steps = 15)]
    fn control_run_mbt_default_config() -> impl Driver {
        MbtDriver::default()
    }

    #[quint_test(spec = "spec/bank.qnt", main = "bank_as", test = "AsDepositThenWithdraw")]
    fn green_named_as_instance() -> impl Driver {
        AsDriver::default()
    }

    // ---------------------------------------------------------------- path probes
    #[quint_test(spec = "spec/bank.qnt", main = "bank_ok", test = "DepositThenWithdraw")]
    fn probe_bare_path() -> impl Driver {
        BareDriver::default()
    }

    #[quint_test(spec = "spec/bank.qnt", main = "bank_ok", test = "DepositThenWithdraw")]
    fn probe_segmented_path() -> impl Driver {
        SegmentedDriver::default()
    }

    // ---------------------------------------------------------------- single-instance
    #[quint_test(spec = "spec/bank.qnt", main = "bank_mc", test = "DepositThenWithdraw")]
    fn probe_non_recording_main_named_run() -> impl Driver {
        McDriver::default()
    }

    #[allow(dead_code)]
    const _: &str = SPEC;
}
