//! Runtime safety monitors evaluated against the REAL implementation state after every rig step — the
//! implementation-side counterparts of the Quint invariants.

use std::collections::BTreeMap;

use viewstamp_proto::Message;

use crate::rig::Rig;

#[derive(Default, Debug)]
pub struct Monitor {
  /// op -> label first seen applied anywhere (committed-op agreement / durability oracle)
  pub committed: BTreeMap<u64, u64>,
  /// (epoch, view) -> the node that advertised a commit as primary (Commit heartbeats)
  pub committers: BTreeMap<(u64, u64), u16>,
  pub violations: Vec<String>,
  seen_net: usize,
}

impl Monitor {
  pub fn observe(&mut self, rig: &Rig, ctx: &str) {
    // (1) agreement / durability: no two nodes ever apply different client ops at the same op number
    for id in 0..rig.nodes.len() as u16 {
      for (op, label) in rig.applied_labels(id) {
        match self.committed.get(&op) {
          Some(&l) if l != label => self.violations.push(format!(
            "[{ctx}] AGREEMENT: n{id} applied label {label} at op {op}, but label {l} was committed there"
          )),
          Some(_) => {}
          None => {
            self.committed.insert(op, label);
          }
        }
      }
    }
    // (2) single committing primary per (epoch, view); (3) learners never vote
    for env in &rig.net[self.seen_net..] {
      match &env.msg {
        Message::Commit(c) => {
          let key = (c.epoch().get(), c.view().get());
          match self.committers.get(&key) {
            Some(&p) if p != env.from => self.violations.push(format!(
              "[{ctx}] SINGLE-PRIMARY: n{} and n{p} both advertised commits in (epoch {}, view {})",
              env.from, key.0, key.1
            )),
            Some(_) => {}
            None => {
              self.committers.insert(key, env.from);
            }
          }
        }
        Message::PrepareOk(_) | Message::StartViewChange(_) | Message::DoViewChange(_)
          if !env.from_was_voter =>
        {
          self.violations.push(format!(
            "[{ctx}] LEARNER-VOTE: n{} (non-voter) emitted {}",
            env.from,
            env.msg.kind_str()
          ));
        }
        _ => {}
      }
    }
    self.seen_net = rig.net.len();
    // (4) fail-stop: an implementation assertion fired
    for (id, p) in &rig.panics {
      let m = format!("[{ctx}] FAIL-STOP: n{id} panicked: {p}");
      if !self.violations.contains(&m) {
        self.violations.push(m);
      }
    }
  }

  /// Every op ever committed must still be applied, in place, on `holder` (used after convergence).
  pub fn missing_on(&self, rig: &Rig, holder: u16) -> Vec<u64> {
    let have: BTreeMap<u64, u64> = rig.applied_labels(holder).into_iter().collect();
    self
      .committed
      .iter()
      .filter(|(op, l)| have.get(op) != Some(l))
      .map(|(op, _)| *op)
      .collect()
  }
}
