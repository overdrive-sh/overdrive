//! quint-connect model-based conformance: replay `spec/vr_sc.qnt` traces against real viewstamp endpoints.
//!
//! * `run_*`   — scripted counterexample runs (`quint test`): the model-checker-found phantom-primary
//!               scenario replayed step by step against the implementation.
//! * `sim_*`   — random traces (`quint run`) per topology, including add/promote/demote/remove churn
//!               under view changes and crashes.
//!
//! Findings (implementation-side monitor violations) are printed; run with `--no-capture`.
use std::sync::{Arc, Mutex};

use quint_connect::runner::{Config, RunConfig, TestConfig, run_test};
use vs_reconfig_conformance::driver::{Findings, Replay, T3v1l, T4v, TSolo, Topology};

fn report(name: &str, r: &quint_connect::Result, f: &Findings) {
  println!("==== {name}: replay result = {}", match r { Ok(()) => "CONFORMS (every step matched)".to_string(), Err(e) => format!("DIVERGED: {e:#}") });
  println!("     traces={} steps={}", f.traces, f.steps);
  for n in &f.notes {
    println!("     note: {n}");
  }
  if f.violations.is_empty() {
    println!("     implementation monitors: no violation");
  } else {
    for v in &f.violations {
      println!("     IMPLEMENTATION FINDING: {v}");
    }
  }
  println!("     final implementation state:\n{}", f.last_status);
  println!("     client replies (client, request, label): {:?}", f.replies);
}

fn scripted<T: Topology>(main: &str, test: &str) -> (quint_connect::Result, Findings) {
  let sink = Arc::new(Mutex::new(Findings::default()));
  let cfg = Config {
    test_name: test.to_string(),
    gen_config: TestConfig {
      spec: "spec/vr_sc.qnt".to_string(),
      main: Some(main.to_string()),
      test: test.to_string(),
      max_samples: Some(1),
      seed: "0x1".to_string(),
    },
  };
  let r = run_test(Replay::<T>::new(sink.clone()), cfg);
  let f = std::mem::take(&mut *sink.lock().unwrap());
  report(test, &r, &f);
  (r, f)
}

fn random<T: Topology>(main: &str, samples: usize, steps: usize, seed: &str) -> (quint_connect::Result, Findings) {
  random_step::<T>(main, None, samples, steps, seed)
}

fn random_step<T: Topology>(
  main: &str,
  step: Option<&str>,
  samples: usize,
  steps: usize,
  seed: &str,
) -> (quint_connect::Result, Findings) {
  let sink = Arc::new(Mutex::new(Findings::default()));
  let cfg = Config {
    test_name: format!("{main}/random"),
    gen_config: RunConfig {
      spec: "spec/vr_sc.qnt".to_string(),
      main: Some(main.to_string()),
      init: None,
      step: step.map(str::to_string),
      max_samples: Some(samples),
      max_steps: Some(steps),
      seed: seed.to_string(),
    },
  };
  let r = run_test(Replay::<T>::new(sink.clone()), cfg);
  let f = std::mem::take(&mut *sink.lock().unwrap());
  report(&format!("{main} random step={} seed={seed}", step.unwrap_or("step")), &r, &f);
  (r, f)
}

#[test]
fn run_phantom_prefix() {
  let (r, _f) = scripted::<T4v>("vr_sc_4v", "phantomPrefix");
  r.unwrap();
}

#[test]
fn run_phantom_fail_stop() {
  let (r, _f) = scripted::<T4v>("vr_sc_4v", "phantomFailStop");
  r.unwrap();
}

#[test]
fn run_phantom_loss() {
  let (r, _f) = scripted::<T4v>("vr_sc_4v", "phantomLoss");
  r.unwrap();
}

fn env_or(k: &str, d: &str) -> String {
  std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[test]
fn sim_4v() {
  let (r, _f) = random::<T4v>("vr_sc_4v", env_or("MBT_SAMPLES", "20").parse().unwrap(), env_or("MBT_STEPS", "30").parse().unwrap(), &env_or("MBT_SEED", "0x2a"));
  r.unwrap();
}

#[test]
fn sim_3v1l() {
  let (r, _f) = random::<T3v1l>("vr_sc_3v1l", env_or("MBT_SAMPLES", "20").parse().unwrap(), env_or("MBT_STEPS", "30").parse().unwrap(), &env_or("MBT_SEED", "0x2a"));
  r.unwrap();
}

#[test]
fn sim_solo() {
  let (r, _f) = random::<TSolo>("vr_sc_solo", env_or("MBT_SAMPLES", "20").parse().unwrap(), env_or("MBT_STEPS", "30").parse().unwrap(), &env_or("MBT_SEED", "0x2a"));
  r.unwrap();
}

// Churn-focused random traces (`stepChurn`: at most 2 crashes per trace, so walks get deep enough for
// add -> bootstrap -> sync -> proof -> promote -> demote -> remove under view changes).
fn churn<T: Topology>(main: &str) {
  let (r, _f) = random_step::<T>(
    main,
    Some("stepChurn"),
    env_or("MBT_SAMPLES", "20").parse().unwrap(),
    env_or("MBT_STEPS", "30").parse().unwrap(),
    &env_or("MBT_SEED", "0x2a"),
  );
  r.unwrap();
}

#[test]
fn churn_4v() {
  churn::<T4v>("vr_sc_4v");
}

#[test]
fn churn_3v1l() {
  churn::<T3v1l>("vr_sc_3v1l");
}

#[test]
fn churn_solo() {
  churn::<TSolo>("vr_sc_solo");
}

// Churn from a GROWN cluster: the scripted grow prefix (`grow*`) then a random `stepChurn` suffix. `quint
// test` writes ONE trace per test, so each sample is a separate replay with its own seed.
fn grow_churn<T: Topology>(main: &str, test: &str) {
  let n: u64 = env_or("MBT_SAMPLES", "20").parse().unwrap();
  let base = u64::from_str_radix(env_or("MBT_SEED", "0x2a").trim_start_matches("0x"), 16).unwrap();
  let mut findings = 0;
  for i in 0..n {
    let seed = format!("0x{:x}", base + i);
    let sink = Arc::new(Mutex::new(Findings::default()));
    let cfg = Config {
      test_name: format!("{test}@{seed}"),
      gen_config: TestConfig {
        spec: "spec/vr_sc.qnt".to_string(),
        main: Some(main.to_string()),
        test: test.to_string(),
        max_samples: Some(1),
        seed: seed.clone(),
      },
    };
    let r = run_test(Replay::<T>::new(sink.clone()), cfg);
    let f = std::mem::take(&mut *sink.lock().unwrap());
    if r.is_err() || !f.violations.is_empty() {
      report(&format!("{main}/{test} seed={seed}"), &r, &f);
      findings += 1;
    }
    r.unwrap();
  }
  println!("==== {main}/{test}: {n} seeded samples replayed from 0x{base:x}; samples with findings: {findings}");
}

#[test]
fn grow_churn_4v() {
  grow_churn::<T4v>("vr_sc_4v", "grow4vChurn");
}

#[test]
fn grow_churn_3v1l() {
  grow_churn::<T3v1l>("vr_sc_3v1l", "grow3v1lChurn");
}

#[test]
fn grow_churn_solo() {
  grow_churn::<TSolo>("vr_sc_solo", "growSoloChurn");
}
