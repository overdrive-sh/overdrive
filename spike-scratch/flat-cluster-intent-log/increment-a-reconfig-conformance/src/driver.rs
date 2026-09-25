//! The quint-connect driver: replays `spec/vr_sc.qnt` traces (random `quint run` traces and scripted
//! `quint test` runs) against REAL viewstamp endpoints held by the [`Rig`], one protocol step at a time,
//! and compares a per-node projection of the implementation state with the spec state after every step.
//!
//! Step mapping (spec action -> implementation input):
//!   ClientRequest{p}          a fresh client's Request delivered to node p
//!   Deliver*{m}               the implementation envelope with the same (kind, src, dst, view, epoch, op)
//!                             — the oldest undelivered one, else a re-delivery (duplicate) of a delivered one
//!   Heartbeat{p}              p's clock advanced by COMMIT_HEARTBEAT (50 ms) -> its Commit heartbeat
//!   Timeout{n}                n's timers fired until n emits a StartViewChange
//!   CatchUpView{n,p}          a higher-view message p->n, then n's GetView -> p, then p's StartView -> n
//!   Crash{n} / Restart{n}     drop the endpoint / `recover_with_reconfig` over its own media
//!   RequestProof{p,l}         `propose_membership(PromoteLearner(l))` MUST return ProofPending
//!   Propose{p,kind,who,ack}   `propose_membership(delta, ack)` MUST mint
//!   Bootstrap{x,d}            format x with d's installed membership (issue #84 interim bootstrap)
//!   SyncFrom{n,d,cp}          only n<->d traffic until n installs d's epoch (the crossing state sync)
//!   CompleteSync{n,d}         complete n's ARMED same-epoch sync: only n<->d sync traffic
//!   RepairExchange / RecoverHead   body repair and the RecoveringHead exit
//!   ProposeRefused / Idle     an f-reducing delta without ack must be refused / no input
//!
//! Compared per node: status, epoch, voters, learners, view, log_view, op (log head), commit.

use core::time::Duration;
use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use itf::Value;
use serde::Deserialize;
use viewstamp_proto::{MemberId, Message, ProposeMembershipError, SingleVoterDelta};

use crate::monitor::Monitor;
use crate::rig::{Envelope, Rig, describe};

/// A topology instance of the spec (`main` module) and its implementation twin.
pub trait Topology: 'static {
  const N_NODES: u16;
  const VOTERS: &'static [u16];
  const LEARNERS: &'static [u16];
  const CHECKPOINT_OPS: u64;
  /// ITF path to the `lastAction` variant (instance-qualified variable name).
  const LAST_ACTION: &'static [&'static str];
  /// Instance-qualified name of the `st` variable.
  const ST: &'static str;
}

pub struct T4v;
impl Topology for T4v {
  const N_NODES: u16 = 4;
  const VOTERS: &'static [u16] = &[0, 1, 2, 3];
  const LEARNERS: &'static [u16] = &[];
  const CHECKPOINT_OPS: u64 = 4;
  const LAST_ACTION: &'static [&'static str] = &["vr_sc_4v::vr_sc::lastAction"];
  const ST: &'static str = "vr_sc_4v::vr_sc::st";
}

pub struct T3v1l;
impl Topology for T3v1l {
  const N_NODES: u16 = 5;
  const VOTERS: &'static [u16] = &[0, 1, 2];
  const LEARNERS: &'static [u16] = &[3];
  const CHECKPOINT_OPS: u64 = 4;
  const LAST_ACTION: &'static [&'static str] = &["vr_sc_3v1l::vr_sc::lastAction"];
  const ST: &'static str = "vr_sc_3v1l::vr_sc::st";
}

pub struct TSolo;
impl Topology for TSolo {
  const N_NODES: u16 = 3;
  const VOTERS: &'static [u16] = &[0];
  const LEARNERS: &'static [u16] = &[];
  const CHECKPOINT_OPS: u64 = 4;
  const LAST_ACTION: &'static [&'static str] = &["vr_sc_solo::vr_sc::lastAction"];
  const ST: &'static str = "vr_sc_solo::vr_sc::st";
}

/// The spec's message record (the fields the driver keys a delivery on).
#[derive(Clone, Debug, Deserialize)]
pub struct SpecMsg {
  pub kind: String,
  pub src: i64,
  pub dst: i64,
  pub view: i64,
  pub epoch: i64,
  pub op: i64,
  pub commit: i64,
  /// the sender's checkpoint_op at emission (Prepare / Commit)
  #[serde(default)]
  pub ckpt: i64,
}

// ------------------------------------------------------------------------------------------------
// The compared state
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeView {
  pub status: String,
  pub epoch: i64,
  pub voters: Vec<i64>,
  pub learners: Vec<i64>,
  pub view: i64,
  pub log_view: i64,
  pub op: i64,
  pub commit: i64,
  pub commit_max: i64,
}

impl NodeView {
  fn only(status: &str) -> Self {
    Self {
      status: status.to_string(),
      epoch: -1,
      voters: vec![],
      learners: vec![],
      view: -1,
      log_view: -1,
      op: -1,
      commit: -1,
      commit_max: -1,
    }
  }
}

/// Projection compared after every step. (Deserialize is required by quint-connect's trait bound; the
/// spec side is parsed by the custom `from_spec` below.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proj(pub BTreeMap<i64, NodeView>);

impl<'de> Deserialize<'de> for Proj {
  fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
    Err(serde::de::Error::custom("Proj is built by from_spec"))
  }
}

fn rec<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
  match v {
    Value::Record(r) => r.get(k).ok_or_else(|| anyhow!("missing field {k}")),
    _ => bail!("expected record for field {k}"),
  }
}

fn int(v: &Value) -> Result<i64> {
  match v {
    Value::Number(n) => Ok(*n),
    Value::BigInt(b) => Ok(b.to_string().parse()?),
    other => bail!("expected int, got {other:?}"),
  }
}

fn boolean(v: &Value) -> Result<bool> {
  match v {
    Value::Bool(b) => Ok(*b),
    other => bail!("expected bool, got {other:?}"),
  }
}

fn string(v: &Value) -> Result<String> {
  match v {
    Value::String(s) => Ok(s.clone()),
    other => bail!("expected string, got {other:?}"),
  }
}

fn ints(v: &Value) -> Result<Vec<i64>> {
  match v {
    Value::List(l) => l.iter().map(int).collect(),
    other => bail!("expected list, got {other:?}"),
  }
}

fn list_len(v: &Value) -> Result<i64> {
  match v {
    Value::List(l) => Ok(l.len() as i64),
    other => bail!("expected list, got {other:?}"),
  }
}

pub fn spec_proj(state: &Value, st_name: &str) -> Result<Proj> {
  let st = rec(state, st_name)?;
  let Value::Map(m) = st else { bail!("st is not a map") };
  let mut out = BTreeMap::new();
  for (k, ns) in m.iter() {
    let id = int(k)?;
    let started = boolean(rec(ns, "started")?)?;
    let up = boolean(rec(ns, "up")?)?;
    let status = string(rec(ns, "status")?)?;
    let nv = if !started {
      NodeView::only("absent")
    } else if status == "retired" {
      NodeView::only("retired")
    } else if !up {
      NodeView::only("down")
    } else {
      let cfg = rec(ns, "cfg")?;
      NodeView {
        status,
        epoch: int(rec(cfg, "epoch")?)?,
        voters: ints(rec(cfg, "voters")?)?,
        learners: ints(rec(cfg, "learners")?)?,
        view: int(rec(ns, "view")?)?,
        log_view: int(rec(ns, "logView")?)?,
        op: list_len(rec(ns, "log")?)?,
        commit: int(rec(ns, "commit")?)?,
        commit_max: int(rec(ns, "cmax")?)?,
      }
    };
    out.insert(id, nv);
  }
  Ok(Proj(out))
}

pub fn impl_proj(rig: &Rig) -> Proj {
  let mut out = BTreeMap::new();
  for id in 0..rig.nodes.len() as u16 {
    let n = &rig.nodes[id as usize];
    let o = rig.obs(id);
    let nv = if !n.started {
      NodeView::only("absent")
    } else if n.retired || o.status == "retired" {
      NodeView::only("retired")
    } else if !n.up {
      NodeView::only("down")
    } else {
      NodeView {
        status: match o.status {
          "normal" => "normal".to_string(),
          "view_change" => "vc".to_string(),
          "recovering_head" => "rhead".to_string(),
          other => other.to_string(),
        },
        epoch: o.epoch as i64,
        voters: o.voters.iter().map(|&x| x as i64).collect(),
        learners: o.learners.iter().map(|&x| x as i64).collect(),
        view: o.view as i64,
        log_view: o.log_view as i64,
        op: o.op as i64,
        commit: o.commit as i64,
        commit_max: o.commit_max as i64,
      }
    };
    out.insert(id as i64, nv);
  }
  Proj(out)
}

// ------------------------------------------------------------------------------------------------
// The driver
// ------------------------------------------------------------------------------------------------

pub struct VsDriver<T: Topology> {
  pub rig: Rig,
  pub mon: Monitor,
  pub label: u64,
  pub steps: usize,
  pub log: Vec<String>,
  _t: core::marker::PhantomData<T>,
}

impl<T: Topology> Default for VsDriver<T> {
  fn default() -> Self {
    Self {
      rig: Rig::new(T::N_NODES, T::VOTERS, T::LEARNERS, T::CHECKPOINT_OPS),
      mon: Monitor::default(),
      label: 0,
      steps: 0,
      log: Vec::new(),
      _t: core::marker::PhantomData,
    }
  }
}

fn kind_of(spec_kind: &str) -> &'static str {
  match spec_kind {
    "prepare" => "Prepare",
    "prepareOk" => "PrepareOk",
    "commit" => "Commit",
    "svc" => "StartViewChange",
    "dvc" => "DoViewChange",
    "sv" => "StartView",
    "reqProof" => "RequestLearnerProof",
    "proof" => "LearnerProof",
    _ => "?",
  }
}

fn env_matches(e: &Envelope, m: &SpecMsg) -> bool {
  if e.from as i64 != m.src || e.to as i64 != m.dst {
    return false;
  }
  let (k, v, ep, op) = describe(&e.msg);
  if k != kind_of(&m.kind) {
    return false;
  }
  match &e.msg {
    Message::Commit(_) => v as i64 == m.view && ep as i64 == m.epoch && op as i64 == m.commit,
    Message::RequestLearnerProof(r) => r.epoch().get() as i64 == m.epoch && r.at_op().get() as i64 == m.op,
    Message::LearnerProof(p) => p.epoch().get() as i64 == m.epoch,
    Message::StartViewChange(_) => v as i64 == m.view && ep as i64 == m.epoch,
    _ => v as i64 == m.view && ep as i64 == m.epoch && op as i64 == m.op,
  }
}

/// The carried (commit, checkpoint) fields of a Prepare / Commit, which the loose op-keyed match ignores: a
/// retransmission of the "same" message may carry newer values.
fn prepare_commit_matches(e: &Envelope, m: &SpecMsg) -> bool {
  match &e.msg {
    Message::Prepare(p) => p.commit().get() as i64 == m.commit && p.checkpoint_op().get() as i64 == m.ckpt,
    Message::Commit(c) => c.checkpoint_op().get() as i64 == m.ckpt,
    _ => true,
  }
}

impl<T: Topology> VsDriver<T> {
  fn note(&mut self, s: String) {
    self.log.push(s);
  }

  fn check(&mut self, ctx: &str) {
    self.mon.observe(&self.rig, ctx);
  }

  pub fn init(&mut self) {
    *self = Self::default();
    self.check("init");
  }

  fn deliver_spec(&mut self, m: &SpecMsg) -> Result<()> {
    // A Prepare also carries the primary's commit; the implementation RETRANSMITS a Prepare (heartbeat /
    // prepare timer) with a newer commit, which the spec does not model. Prefer the envelope whose carried
    // commit is EXACTLY the spec's; fall back to the loose (op-keyed) match only if none exists.
    let strict = |e: &Envelope| env_matches(e, m) && prepare_commit_matches(e, m);
    let k = self
      .rig
      .pending(strict)
      .first()
      .copied()
      .or_else(|| self.rig.ever(strict).last().copied())
      .or_else(|| self.rig.pending(|e| env_matches(e, m)).first().copied())
      .or_else(|| self.rig.ever(|e| env_matches(e, m)).last().copied());
    let Some(k) = k else {
      bail!(
        "DIVERGENCE(missing message): the spec delivers {} n{}->n{} (view {}, epoch {}, op {}, commit {}) \
         but the implementation never emitted it\nimpl state:\n{}impl net tail:\n{}",
        m.kind,
        m.src,
        m.dst,
        m.view,
        m.epoch,
        m.op,
        m.commit,
        self.rig.status_line(),
        self.rig.net_trace(self.rig.net.len().saturating_sub(25))
      )
    };
    self.rig.deliver(k);
    Ok(())
  }

  pub fn client_request(&mut self, p: i64) -> Result<()> {
    self.label += 1;
    self.rig.client_request(p as u16, self.label);
    Ok(())
  }

  pub fn heartbeat(&mut self, p: i64) -> Result<()> {
    let p = p as u16;
    let mark = self.rig.net.len();
    // the commit-heartbeat timer self-arms on the first service of an idle primary, then fires
    for _ in 0..4 {
      self.rig.advance(p, Duration::from_millis(50));
      if self.rig.net[mark..].iter().any(|e| e.from == p && matches!(e.msg, Message::Commit(_))) {
        return Ok(());
      }
    }
    bail!("DIVERGENCE(heartbeat): n{p} emitted no Commit heartbeat\n{}", self.rig.status_line())
  }

  /// The idle / view-change-status timeout: `propose_next_view` proposes `view + 1`. Timer services that
  /// only RETRANSMIT the current SVC target (the 100 ms `svc_message` cadence) are passed over.
  pub fn timeout(&mut self, n: i64) -> Result<()> {
    let n = n as u16;
    let target = self.rig.obs(n).view + 1;
    let was_normal = self.rig.obs(n).status == "normal";
    let start_ns = self.rig.nodes[n as usize].now.as_nanos();
    let mark = self.rig.net.len();
    for _ in 0..50 {
      if !self.rig.fire_next_timer(n) {
        break;
      }
      if self.rig.net[mark..].iter().any(|e| {
        e.from == n
          && matches!(&e.msg, Message::StartViewChange(s) if s.view().get() >= target)
      }) {
        // A Normal backup whose svc_target was already raised to view+1 RETRANSMITS that SVC off its
        // svc_message timer (100 ms) without re-checking the quorum; the spec's Timeout is the
        // primary_idle firing (propose_next_view -> join + maybe_start_view_change). Fire every timer
        // due within one PRIMARY_IDLE (200 ms) of the start so the idle timer has certainly fired
        // (re-proposing the same view+1 while still Normal is idempotent); stop once it left Normal.
        if was_normal {
          for _ in 0..50 {
            let next = self.rig.nodes[n as usize].ep.as_ref().and_then(|e| e.poll_timeout());
            let Some(t) = next else { break };
            if t.as_nanos() > start_ns + 200_000_000 || self.rig.obs(n).status != "normal" {
              break;
            }
            self.rig.fire_next_timer(n);
          }
        }
        return Ok(());
      }
    }
    bail!(
      "DIVERGENCE(timeout): n{n}'s timers never produced a StartViewChange\n{}",
      self.rig.status_line()
    )
  }

  pub fn catch_up_view(&mut self, n: i64, p: i64) -> Result<()> {
    let (n, p) = (n as u16, p as u16);
    // a node already in the catch-up posture has a GetView to p in flight: no trigger needed
    let gv0 = |e: &Envelope| e.from == n && e.to == p && matches!(e.msg, Message::GetView(_));
    if !self.rig.pending(gv0).is_empty() {
      let k = self.rig.pending(gv0)[0];
      let mark = self.rig.net.len();
      self.rig.deliver(k);
      let sv = |e: &Envelope| {
        e.seq as usize > mark && e.from == p && e.to == n && matches!(e.msg, Message::StartView(_))
      };
      let Some(k) = self.rig.pending(sv).first().copied() else {
        bail!("DIVERGENCE(catch-up): p{p} did not answer n{n}'s GetView\n{}", self.rig.status_line());
      };
      self.rig.deliver(k);
      return Ok(());
    }
    let nv = self.rig.obs(n).view;
    let ne = self.rig.obs(n).epoch;
    let hv = |e: &Envelope| {
      let (_, v, _, _) = describe(&e.msg);
      let (_, _, ep, _) = describe(&e.msg);
      e.from == p && e.to == n && v > nv && ep == ne
        && matches!(e.msg, Message::Commit(_) | Message::Prepare(_))
    };
    if self.rig.pending(hv).is_empty() {
      // make the primary advertise its view (its Commit heartbeat)
      self.rig.advance(p, Duration::from_millis(50));
    }
    let Some(k) = self.rig.pending(hv).first().copied() else {
      bail!("DIVERGENCE(catch-up): no higher-view message p{p}->n{n}\n{}", self.rig.status_line());
    };
    self.rig.deliver(k);
    // n GetViews p; p answers with its StartView
    let gv = |e: &Envelope| e.from == n && e.to == p && matches!(e.msg, Message::GetView(_));
    let Some(k) = self.rig.pending(gv).first().copied() else {
      bail!("DIVERGENCE(catch-up): n{n} did not GetView p{p}\n{}", self.rig.status_line());
    };
    let mark = self.rig.net.len();
    self.rig.deliver(k);
    let sv = |e: &Envelope| {
      e.seq as usize > mark && e.from == p && e.to == n && matches!(e.msg, Message::StartView(_))
    };
    let Some(k) = self.rig.pending(sv).first().copied() else {
      bail!("DIVERGENCE(catch-up): p{p} did not answer n{n}'s GetView\n{}", self.rig.status_line());
    };
    self.rig.deliver(k);
    Ok(())
  }

  pub fn restart(&mut self, n: i64) -> Result<()> {
    self
      .rig
      .restart(n as u16)
      .map_err(|e| anyhow!("DIVERGENCE(restart): n{n}: {e}"))
  }

  fn delta(kind: &str, who: i64) -> SingleVoterDelta {
    let m = MemberId::new(who as u128);
    match kind {
      "promote" => SingleVoterDelta::PromoteLearner(m),
      "demote" => SingleVoterDelta::DemoteVoter(m),
      "add" => SingleVoterDelta::AddLearner(m),
      _ => SingleVoterDelta::RemoveLearner(m),
    }
  }

  pub fn request_proof(&mut self, p: i64, l: i64) -> Result<()> {
    match self.rig.propose(p as u16, Self::delta("promote", l), false) {
      Some(Err(ProposeMembershipError::ProofPending)) => Ok(()),
      other => bail!(
        "DIVERGENCE(requestProof): spec solicits a proof for PromoteLearner({l}) at n{p}; implementation \
         returned {other:?}\n{}",
        self.rig.status_line()
      ),
    }
  }

  pub fn propose(&mut self, p: i64, kind: &str, who: i64, ack: bool) -> Result<()> {
    match self.rig.propose(p as u16, Self::delta(kind, who), ack) {
      Some(Ok(_)) => Ok(()),
      other => bail!(
        "DIVERGENCE(propose): spec mints {kind}({who}) ack={ack} at n{p}; implementation returned {other:?}\n{}",
        self.rig.status_line()
      ),
    }
  }

  /// An f-reducing delta proposed WITHOUT the acknowledgement token must be refused and mint nothing.
  pub fn propose_refused(&mut self, p: i64, kind: &str, who: i64) -> Result<()> {
    let head = self.rig.obs(p as u16).op;
    match self.rig.propose(p as u16, Self::delta(kind, who), false) {
      Some(Err(ProposeMembershipError::ReducedFaultToleranceUnacknowledged { .. }))
        if self.rig.obs(p as u16).op == head =>
      {
        Ok(())
      }
      other => bail!(
        "DIVERGENCE(proposeRefused): spec expects {kind}({who}) WITHOUT ack at n{p} to be refused; implementation \
         returned {other:?}\n{}",
        self.rig.status_line()
      ),
    }
  }

  pub fn bootstrap(&mut self, x: i64, d: i64) -> Result<()> {
    let m = self
      .rig
      .membership_of(d as u16)
      .ok_or_else(|| anyhow!("bootstrap donor n{d} is down"))?;
    self.rig.format_and_start(x as u16, m);
    Ok(())
  }

  /// The crossing. Triggers: a higher-epoch Commit/Prepare d->n (d's heartbeat, made if d is a primary),
  /// or n's LOWER-epoch vote/lead/status traffic n->d, which d drops but answers with `EpochAhead`. Then
  /// only the state-sync sub-protocol between n and d (RequestSync / SyncCheckpoint / RequestBlock /
  /// BlockResponse / EpochAhead), one message at a time, stopping the moment n installs d's epoch.
  pub fn sync_from(&mut self, n: i64, d: i64, cp: i64) -> Result<()> {
    let (n, d) = (n as u16, d as u16);
    let target = self.rig.obs(d).epoch;
    // (triggers are classified by the epoch the MESSAGE carries, not the sender's epoch at collection: a
    // commit-first swap emits predecessor-epoch messages in the same step that installs the successor)
    let msg_epoch = |e: &Envelope| describe(&e.msg).2;
    let trig_d = move |e: &Envelope| {
      e.from == d && e.to == n && msg_epoch(e) >= target
        && matches!(e.msg, Message::Commit(_) | Message::Prepare(_))
    };
    let trig_n = move |e: &Envelope| {
      e.from == n && e.to == d && msg_epoch(e) < target
        && matches!(
          e.msg,
          Message::Commit(_) | Message::Prepare(_) | Message::StartViewChange(_)
            | Message::DoViewChange(_) | Message::LearnerStatus(_)
        )
    };
    let sync = move |e: &Envelope| {
      ((e.from == n && e.to == d) || (e.from == d && e.to == n))
        && matches!(
          e.msg,
          Message::RequestSync(_) | Message::SyncCheckpoint(_) | Message::RequestBlock(_)
            | Message::BlockResponse(_) | Message::EpochAhead(_)
        )
    };
    // A donor that never exchanges traffic with `n` (e.g. a successor-epoch LEARNER: `n` reports only to
    // its own primary, a learner sends no Commit/Prepare) is reached through the broadcast RequestSync
    // (`send_request_sync` fans out to all Backups) once ANY successor-epoch member has answered `n`'s
    // lower-epoch traffic with EpochAhead. Only n->d / d->n sync traffic is delivered above, so the
    // crossing is still served by `d` alone.
    let ahead = move |e: &Envelope| e.to == n && matches!(e.msg, Message::EpochAhead(_));
    let epochs: Vec<u64> = (0..self.rig.nodes.len() as u16).map(|i| self.rig.obs(i).epoch).collect();
    // ... or once a successor-epoch PRIMARY's own traffic (its Commit heartbeat / Prepare) reaches `n`
    // (maybe_request_cross_epoch_catchup arms a forced cross-epoch sync, broadcast to all Backups).
    let trig_any_d = move |e: &Envelope| {
      e.to == n && e.from != d && msg_epoch(e) >= target
        && matches!(e.msg, Message::Commit(_) | Message::Prepare(_))
    };
    // a higher-epoch Normal primary whose heartbeat `n` will act on (it is a member of n's config and holds n
    // as a member of its own) — the trigger that leaves n's own state untouched, preferred over n's timers
    let n_members: Vec<u16> = self.rig.membership_of(n).map_or(vec![], |m| {
      m.members_slice().iter().map(|x| x.get() as u16).collect()
    });
    let ahead_primary: Option<u16> = (0..self.rig.nodes.len() as u16).find(|&i| {
      let o = self.rig.obs(i);
      o.up && o.is_primary && o.epoch >= target && o.status == "normal" && n_members.contains(&i)
        && self.rig.membership_of(i).is_some_and(|m| m.slot_of(MemberId::new(n as u128)).is_some())
    });
    let trig_any = move |e: &Envelope| {
      e.from == n && msg_epoch(e) < target && epochs.get(e.to as usize).is_some_and(|&x| x >= target)
        && matches!(
          e.msg,
          Message::Commit(_) | Message::Prepare(_) | Message::StartViewChange(_)
            | Message::DoViewChange(_) | Message::LearnerStatus(_)
        )
    };
    if let Some(p) = ahead_primary
      && self.rig.pending(|e| e.from == p && e.to == n && msg_epoch(e) >= target
        && matches!(e.msg, Message::Commit(_) | Message::Prepare(_))).is_empty()
    {
      self.rig.advance(p, Duration::from_millis(50));
    }
    let diag = std::env::var("MBT_TRACE_SYNC").is_ok();
    for _ in 0..600 {
      let o = self.rig.obs(n);
      if o.epoch >= target || self.rig.nodes[n as usize].retired || !self.rig.nodes[n as usize].up {
        break;
      }
      let mark = self.rig.net.len();
      if self.rig.deliver_all_where(1, sync) > 0 {
        if diag {
          for e in &self.rig.net[mark.saturating_sub(1).min(self.rig.net.len())..] {
            match &e.msg {
              Message::SyncCheckpoint(c) => println!(
                "      [sync-diag] SyncCheckpoint n{}->n{} nonce={} ckpt={} epoch={} cfg={} membership_bytes={} install_op={:?}",
                e.from, e.to, c.nonce(), c.checkpoint_op().get(), c.epoch().get(), c.config_id(),
                c.membership().len(), c.config_install_op().map(|o| o.get())
              ),
              Message::RequestSync(r) => println!(
                "      [sync-diag] RequestSync n{}->n{} nonce={} ckpt={} cfg={} recovery={}",
                e.from, e.to, r.nonce(), r.checkpoint_op().get(), r.config_id(), r.recovery()
              ),
              _ => {}
            }
          }
          let ep = self.rig.nodes[n as usize].ep.as_ref();
          println!(
            "      [sync-diag] n{n}: install={:?} checkpoint={:?} fetch_donor={:?} status={:?} emitted:\n{}",
            ep.map(|e| e.debug_install_state()),
            ep.map(|e| e.checkpoint_op().get()),
            ep.and_then(|e| e.block_fetch_donor()),
            ep.map(|e| e.status().as_str()),
            self.rig.net_trace(mark.min(self.rig.net.len()))
          );
        }
        continue;
      }
      if let Some(k) = self.rig.pending(trig_d).first().copied() {
        self.rig.deliver(k);
        continue;
      }
      if let Some(k) = self.rig.pending(trig_n).first().copied() {
        self.rig.deliver(k);
        continue;
      }
      if self.rig.deliver_all_where(1, ahead) > 0 {
        continue;
      }
      if let Some(k) = self.rig.pending(&trig_any).first().copied() {
        self.rig.deliver(k);
        continue;
      }
      if let Some(k) = self.rig.pending(trig_any_d).first().copied() {
        self.rig.deliver(k);
        continue;
      }
      // prefer the heartbeat of a higher-epoch primary (leaves n's state untouched); fall back to n's own
      // timers (its idle SVC / learner status / view-change retransmits) only when there is none
      if let Some(p) = ahead_primary {
        self.rig.advance(p, Duration::from_millis(50));
      } else {
        self.rig.fire_next_timer(n);
      }
    }
    let o = self.rig.obs(n);
    if o.epoch < target && !self.rig.nodes[n as usize].retired {
      bail!(
        "DIVERGENCE(syncFrom): n{n} did not cross into epoch {target} from n{d}\n{}",
        self.rig.status_line()
      );
    }
    self.note(format!("syncFrom n{n}<-n{d}: spec cp={cp}, impl commit={} op={}", o.commit, o.op));
    Ok(())
  }

  /// Completing an ARMED same-epoch state sync of `n` from donor `d`: the sync was armed by an earlier
  /// delivery (the primary's Commit carrying its checkpoint: `maybe_request_sync` / `maybe_force_sync`), which
  /// broadcast a RequestSync. Only n<->d sync traffic is delivered (so `d` alone serves it) until `n` installs
  /// `d`'s checkpoint. n's own timers are NOT fired (their side effects are not part of this step).
  pub fn complete_sync(&mut self, n: i64, d: i64) -> Result<()> {
    let (n, d) = (n as u16, d as u16);
    let cp = self.rig.nodes[d as usize].ep.as_ref().map_or(0, |ep| ep.checkpoint_op().get());
    let sync = move |e: &Envelope| {
      ((e.from == n && e.to == d) || (e.from == d && e.to == n))
        && matches!(
          e.msg,
          Message::RequestSync(_) | Message::SyncCheckpoint(_) | Message::RequestBlock(_)
            | Message::BlockResponse(_)
        )
    };
    for _ in 0..600 {
      let o = self.rig.obs(n);
      if o.commit >= cp || !self.rig.nodes[n as usize].up {
        break;
      }
      if self.rig.deliver_all_where(1, sync) == 0 {
        break;
      }
    }
    let o = self.rig.obs(n);
    if o.commit < cp {
      bail!(
        "DIVERGENCE(completeSync): n{n} did not state-sync to n{d}'s checkpoint {cp}\n{}",
        self.rig.status_line()
      );
    }
    self.note(format!("completeSync n{n}<-n{d}: donor checkpoint={cp}, impl commit={} op={}", o.commit, o.op));
    Ok(())
  }

  /// RecoveringHead exit: `n`'s Recovery (sent at restart / re-solicited by its recover_head timer) to the
  /// primary `p`, then `p`'s RecoveryResponse, which `n` adopts.
  pub fn recover_head(&mut self, n: i64, p: i64) -> Result<()> {
    let (n, p) = (n as u16, p as u16);
    let req = move |e: &Envelope| e.from == n && e.to == p && matches!(e.msg, Message::Recovery(_));
    if self.rig.pending(req).is_empty() {
      for _ in 0..10 {
        if !self.rig.fire_next_timer(n) || !self.rig.pending(req).is_empty() {
          break;
        }
      }
    }
    let Some(k) = self.rig.pending(req).last().copied() else {
      bail!("DIVERGENCE(recoverHead): n{n} has no Recovery for n{p}\n{}", self.rig.status_line());
    };
    let mark = self.rig.net.len();
    self.rig.deliver(k);
    let resp = move |e: &Envelope| {
      e.seq as usize > mark && e.from == p && e.to == n && matches!(e.msg, Message::RecoveryResponse(_))
    };
    let Some(k) = self.rig.pending(resp).first().copied() else {
      bail!("DIVERGENCE(recoverHead): p{p} did not answer n{n}'s Recovery\n{}", self.rig.status_line());
    };
    self.rig.deliver(k);
    Ok(())
  }

  /// Body repair between n and h: n's RequestPrepare(-Range) to h, then h's repair-serve responses and
  /// nacks back to n. If n's earlier requests to h were already consumed, n's repair-retry timer
  /// re-solicits.
  pub fn repair_exchange(&mut self, n: i64, h: i64) -> Result<()> {
    let (n, h) = (n as u16, h as u16);
    // registered repair-hole solicitations only (RequestPrepareRange / RequestPrepare at or below the head);
    // a tail-gap RequestPrepare (op above the head at emission) is below the spec's abstraction
    let req = move |e: &Envelope| {
      e.from == n
        && e.to == h
        && !e.tail_gap
        && matches!(e.msg, Message::RequestPrepare(_) | Message::RequestPrepareRange(_))
    };
    if self.rig.pending(req).is_empty() {
      for _ in 0..10 {
        if !self.rig.fire_next_timer(n) || !self.rig.pending(req).is_empty() {
          break;
        }
      }
    }
    if self.rig.pending(req).is_empty() {
      bail!(
        "DIVERGENCE(repair): n{n} has no repair request for n{h} (spec: n{n} holds header-only ops n{h} \
         can serve or nack)\n{}",
        self.rig.status_line()
      );
    }
    let resp = move |e: &Envelope| {
      e.from == h && e.to == n && (e.aux || matches!(e.msg, Message::Nack(_) | Message::RepairBatch(_)))
    };
    for _ in 0..50 {
      let a = self.rig.deliver_all_where(64, req);
      let b = self.rig.deliver_all_where(64, resp);
      if a + b == 0 {
        break;
      }
    }
    Ok(())
  }

  pub fn deliver(&mut self, m: SpecMsg) -> Result<()> {
    self.deliver_spec(&m)
  }

  pub fn after_step(&mut self, ctx: &str) -> Result<()> {
    self.steps += 1;
    self.check(ctx);
    Ok(())
  }
}

// ------------------------------------------------------------------------------------------------
// quint-connect glue
// ------------------------------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use quint_connect::{Config as QcConfig, Driver, State, Step, switch};

/// What a replay learned about the IMPLEMENTATION (monitor violations, notes), readable after
/// `run_test` consumed the driver.
#[derive(Default, Debug)]
pub struct Findings {
  pub violations: Vec<String>,
  pub notes: Vec<String>,
  pub steps: usize,
  pub traces: usize,
  pub last_status: String,
  /// every Reply the implementation sent to a client: (client, request, applied label)
  pub replies: Vec<(u128, u64, u64)>,
}

/// The driver quint-connect owns; shares its findings through `sink`.
pub struct Replay<T: Topology> {
  pub d: VsDriver<T>,
  pub sink: Arc<Mutex<Findings>>,
}

impl<T: Topology> Replay<T> {
  pub fn new(sink: Arc<Mutex<Findings>>) -> Self {
    Self { d: VsDriver::default(), sink }
  }

  fn flush(&mut self, ctx: &str) {
    let mut f = self.sink.lock().unwrap();
    for v in self.d.mon.violations.drain(..) {
      if !f.violations.contains(&v) {
        f.violations.push(v);
      }
    }
    for n in self.d.log.drain(..) {
      f.notes.push(format!("[{ctx}] {n}"));
    }
    f.steps += 1;
    f.last_status = self.d.rig.status_line();
    f.replies = self
      .d
      .rig
      .replies
      .iter()
      .map(|(c, r, _)| (*c, *r, (*c - 1_000_000) as u64))
      .collect();
  }
}

impl<T: Topology> State<Replay<T>> for Proj {
  fn from_driver(r: &Replay<T>) -> quint_connect::Result<Self> {
    Ok(impl_proj(&r.d.rig))
  }

  fn from_spec(value: Value) -> quint_connect::Result<Self> {
    spec_proj(&value, T::ST)
  }
}

impl<T: Topology> Driver for Replay<T> {
  type State = Proj;

  fn config() -> QcConfig {
    QcConfig { state: &[], nondet: T::LAST_ACTION }
  }

  fn step(&mut self, step: &Step) -> quint_connect::Result {
    let ctx = step.action_taken.clone();
    let trace = std::env::var("MBT_TRACE").is_ok();
    let mark = self.d.rig.net.len();
    let emark = self.d.rig.events.len();
    let r: quint_connect::Result = (|| {
      switch!(step {
        Init => {
          self.d.init();
          self.sink.lock().unwrap().traces += 1;
        },
        ClientRequest(p: i64) => self.d.client_request(p)?,
        DeliverPrepare(m: SpecMsg) => self.d.deliver(m)?,
        DeliverPrepareOk(m: SpecMsg) => self.d.deliver(m)?,
        DeliverCommit(m: SpecMsg) => self.d.deliver(m)?,
        Heartbeat(p: i64) => self.d.heartbeat(p)?,
        Timeout(n: i64) => self.d.timeout(n)?,
        DeliverSvc(m: SpecMsg) => self.d.deliver(m)?,
        DeliverDvc(m: SpecMsg) => self.d.deliver(m)?,
        DeliverSv(m: SpecMsg) => self.d.deliver(m)?,
        CatchUpView(n: i64, p: i64) => self.d.catch_up_view(n, p)?,
        Crash(n: i64) => self.d.rig.crash(n as u16),
        Restart(n: i64) => self.d.restart(n)?,
        RequestProof(p: i64, l: i64) => self.d.request_proof(p, l)?,
        DeliverReqProof(m: SpecMsg) => self.d.deliver(m)?,
        DeliverProof(m: SpecMsg) => self.d.deliver(m)?,
        Propose(p: i64, kind: String, who: i64, ack: bool) => self.d.propose(p, &kind, who, ack)?,
        Bootstrap(x: i64, d: i64) => self.d.bootstrap(x, d)?,
        SyncFrom(n: i64, d: i64, cp: i64) => self.d.sync_from(n, d, cp)?,
        RepairExchange(n: i64, h: i64) => self.d.repair_exchange(n, h)?,
        CompleteSync(n: i64, d: i64) => self.d.complete_sync(n, d)?,
        RecoverHead(n: i64, p: i64) => self.d.recover_head(n, p)?,
        Idle => {},
        ProposeRefused(p: i64, kind: String, who: i64) => self.d.propose_refused(p, &kind, who)?
      })
    })();
    let _ = self.d.after_step(&ctx);
    if trace {
      println!("   >> impl after {ctx}:\n{}{}", self.d.rig.net_trace(mark.min(self.d.rig.net.len())), self.d.rig.status_line());
      for (n, e) in &self.d.rig.events[emark.min(self.d.rig.events.len())..] {
        println!("    event n{n}: {e:?}");
      }
    }
    self.flush(&ctx);
    r
  }
}
