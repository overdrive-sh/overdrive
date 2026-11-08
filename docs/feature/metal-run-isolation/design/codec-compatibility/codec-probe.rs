// Bounded codec diagnostic only. State=() deliberately disables conformance checks.
// The real installed public runner generates traces and constructs private Step values.
use quint_connect::{Config, Driver, Result, Step};
use quint_connect::runner::{self, RunConfig, TestConfig};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

struct CodecProbe(Arc<Mutex<Vec<serde_json::Value>>>);
impl Driver for CodecProbe {
    type State = ();
    fn config() -> Config {
        Config { state: &[], nondet: &["runner_conf::runner_protocol::conf_step"] }
    }
    fn step(&mut self, step: &Step) -> Result {
        let actor = i64::deserialize(step.nondet_picks.get("actor").expect("actor pick").clone())?;
        let value = i64::deserialize(step.nondet_picks.get("value").expect("value pick").clone())?;
        self.0.lock().unwrap().push(serde_json::json!({
            "action": step.action_taken, "actor": actor, "value": value
        }));
        Ok(())
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert_eq!(a.len(), 5, "SPEC named:RUN|random OUTPUT SEED");
    let steps = Arc::new(Mutex::new(Vec::new()));
    let driver = CodecProbe(steps.clone());
    let result = if a[2] == "random" {
        runner::run_test(driver, runner::Config {
            test_name: "codec-only bounded random".into(),
            gen_config: RunConfig { spec:a[1].clone(),main:Some("runner_conf".into()),
                init:None,step:None,max_samples:Some(4),max_steps:Some(32),seed:a[4].clone() }
        })
    } else {
        runner::run_test(driver, runner::Config {
            test_name: format!("codec-only {}",a[2]),
            gen_config: TestConfig { spec:a[1].clone(),main:Some("runner_conf".into()),
                test:a[2].strip_prefix("named:").unwrap().into(),max_samples:Some(1),seed:a[4].clone() }
        })
    };
    let report = serde_json::json!({"codec_only":true,"state_comparison":false,
        "seed":a[4],"mode":a[2],"ok":result.is_ok(),
        "error":result.as_ref().err().map(|e|format!("{e:#}")),
        "steps_reaching_driver":*steps.lock().unwrap()});
    std::fs::write(&a[3],serde_json::to_string_pretty(&report).unwrap()+"\n").unwrap();
    if let Err(e)=result {eprintln!("{e:#}");std::process::exit(2)}
}
