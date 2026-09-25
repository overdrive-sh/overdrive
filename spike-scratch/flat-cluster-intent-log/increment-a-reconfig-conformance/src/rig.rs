//! The rig: N `Endpoint<SimSm, SingleChange>` + a virtual network under explicit, per-message control.

use std::panic::{AssertUnwindSafe, catch_unwind};

use bytes::Bytes;
use core::time::Duration;
use viewstamp_proto::{
  AcceptReducedFaultTolerance, BlockJobCursor, ClientId, Config, Endpoint, Epoch, Event, Instant,
  MemberId, Membership, Message, OpNumber, Peer, ProposeMembershipError, Recipient, Recovered, ReplicaId,
  Request, RequestNumber, SingleChange, SingleVoterDelta, Status, Storage, execute_block_job,
};
use viewstamp_simulation::sm::{LogSm, SimSm};
use viewstamp_simulation::storage::Shared;
use viewstamp_simulation::{InMemorySuperblock, InMemoryWal, MemBlockStore};

pub type Ep = Endpoint<SimSm, SingleChange>;
type St = Storage<Shared<InMemoryWal>, Shared<InMemorySuperblock>, SimSm>;

const CLUSTER: u128 = 1;

/// One node: its endpoint (None while crashed / never started) and its durable media.
pub struct Node {
  pub id: u16,
  pub ep: Option<Ep>,
  wal: Shared<InMemoryWal>,
  sb: Shared<InMemorySuperblock>,
  storage: St,
  blocks: MemBlockStore,
  lane: BlockJobCursor,
  pub now: Instant,
  pub started: bool,
  pub up: bool,
  pub retired: bool,
  pub incarnation: u64,
  /// Whether the node was a voter when the CURRENT input started: a swap applied while handling the
  /// input (commit-first install at a Reconfigure op) can demote it after it already emitted a vote,
  /// and outputs are collected only after the input + its storage completions are processed.
  pre_voter: bool,
  /// epoch -> whether the node was a voter at any point while it held that epoch's configuration (sampled
  /// before and after every endpoint call). A vote is attributed to the role in the MESSAGE's epoch: a
  /// crossing input can emit a (stale) predecessor-epoch vote and then install a successor where the node
  /// is a learner, and `pre_voter` alone (re-sampled per storage completion) would misattribute it.
  voter_in: std::collections::BTreeMap<u64, bool>,
  pub panicked: Option<String>,
  /// The last observed applied history `(op, body)` (kept across a crash for observation).
  pub applied: Vec<(u64, Bytes)>,
}

/// A replica->replica message the network carried. Never removed: `delivered` marks consumption, and a
/// trace may re-deliver any envelope (duplication).
#[derive(Clone, Debug)]
pub struct Envelope {
  pub seq: u64,
  pub from: u16,
  pub to: u16,
  pub msg: Message,
  pub delivered: bool,
  /// Whether the emitter was a voter when it emitted (the "learners never vote" monitor input).
  pub from_was_voter: bool,
  /// The emitter's (epoch, view, is_primary) at emission.
  pub from_epoch: u64,
  pub from_view: u64,
  /// Emitted while serving a body-repair request (a repair-serve `Prepare` / `RepairBatch`).
  pub aux: bool,
  /// A `RequestPrepare` for an op ABOVE the sender's head at emission: a tail-gap fetch
  /// (`request_tail_gap`), not a registered repair-hole solicitation (below the spec's abstraction).
  pub tail_gap: bool,
}

/// A compact, comparable view of one node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obs {
  pub id: u16,
  pub started: bool,
  pub up: bool,
  pub status: &'static str,
  pub epoch: u64,
  pub voters: Vec<u16>,
  pub learners: Vec<u16>,
  pub view: u64,
  pub log_view: u64,
  pub op: u64,
  pub commit: u64,
  pub commit_max: u64,
  pub is_primary: bool,
}

pub struct Rig {
  pub nodes: Vec<Node>,
  pub net: Vec<Envelope>,
  pub replies: Vec<(u128, u64, Bytes)>,
  pub events: Vec<(u16, Event)>,
  seq: u64,
  checkpoint_ops: u64,
  genesis: Membership,
  pub panics: Vec<(u16, String)>,
}

fn seed(id: u16, inc: u64) -> u64 {
  ((id as u64) << 32) ^ inc.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5EED
}

fn member_ids(m: &Membership) -> (Vec<u16>, Vec<u16>) {
  let all: Vec<u16> = m.members_slice().iter().map(|x| x.get() as u16).collect();
  let rc = m.replica_count() as usize;
  (all[..rc].to_vec(), all[rc..].to_vec())
}

fn panic_msg(p: Box<dyn std::any::Any + Send>) -> String {
  if let Some(s) = p.downcast_ref::<&str>() {
    (*s).to_string()
  } else if let Some(s) = p.downcast_ref::<String>() {
    s.clone()
  } else {
    "<non-string panic>".to_string()
  }
}

impl Rig {
  /// A cluster over node ids `0..n_nodes`; the genesis membership is `voters` + `learners` (ordered
  /// slots). Nodes absent from the genesis membership exist as media only (`started == false`).
  pub fn new(n_nodes: u16, voters: &[u16], learners: &[u16], checkpoint_ops: u64) -> Self {
    let members: Vec<MemberId> = voters
      .iter()
      .chain(learners.iter())
      .map(|&x| MemberId::new(x as u128))
      .collect();
    // config_id 0, exactly like the simulation harness's genesis (`from_durable_parts`).
    let genesis = Membership::from_durable_parts(
      Epoch::new(0),
      voters.len() as u8,
      learners.len() as u16,
      members,
      0,
    )
    .expect("valid genesis membership");
    let mut rig = Self {
      nodes: Vec::new(),
      net: Vec::new(),
      replies: Vec::new(),
      events: Vec::new(),
      seq: 0,
      checkpoint_ops,
      genesis: genesis.clone(),
      panics: Vec::new(),
    };
    for id in 0..n_nodes {
      let wal = Shared::new(InMemoryWal::new());
      let sb = Shared::new(InMemorySuperblock::new());
      let storage = Storage::new(wal.clone(), sb.clone());
      rig.nodes.push(Node {
        id,
        ep: None,
        wal,
        sb,
        storage,
        blocks: MemBlockStore::new_gc_disabled(),
        lane: BlockJobCursor::new(),
        now: Instant::from_nanos(0),
        started: false,
        up: false,
        retired: false,
        incarnation: 0,
        pre_voter: false,
        voter_in: std::collections::BTreeMap::new(),
        panicked: None,
        applied: Vec::new(),
      });
    }
    for id in 0..n_nodes {
      if genesis.slot_of(MemberId::new(id as u128)).is_some() {
        rig.format_and_start(id, genesis.clone());
      }
    }
    rig
  }

  fn config(&self, id: u16) -> Config {
    Config::with_checkpoint_ops(CLUSTER, MemberId::new(id as u128), self.checkpoint_ops)
      .expect("valid config")
  }

  /// Genesis-format node `id`'s (virgin) store with `membership` and start it. Used for the genesis
  /// members and for the out-of-band bootstrap of a brand-new learner (viewstamp issue #84 interim path).
  pub fn format_and_start(&mut self, id: u16, membership: Membership) {
    let cfg = self.config(id);
    let n = &mut self.nodes[id as usize];
    let ep = Endpoint::<SimSm, SingleChange>::with_reconfig(
      cfg,
      membership,
      seed(id, 0),
      SimSm::Plain(LogSm::default()),
      u64::MAX,
    )
    .commit(&n.wal, &mut n.sb)
    .expect("genesis commit formats the virgin store");
    n.ep = Some(ep);
    n.started = true;
    n.up = true;
    self.pump(id);
  }

  pub fn membership_of(&self, id: u16) -> Option<Membership> {
    self.nodes[id as usize].ep.as_ref().map(|e| e.membership_clone())
  }

  // ------------------------------------------------------------------------------------------------
  // The embedding loop
  // ------------------------------------------------------------------------------------------------

  /// Run `f` on node `id`'s endpoint; a panic inside the endpoint is a FAIL-STOP: recorded, and the node
  /// is taken down (its media survive, exactly like a process crash).
  fn guarded<R>(&mut self, id: u16, f: impl FnOnce(&mut Ep, &mut St, Instant) -> R) -> Option<R> {
    let n = &mut self.nodes[id as usize];
    let now = n.now;
    let ep = n.ep.as_mut()?;
    n.pre_voter = ep.is_voter();
    *n.voter_in.entry(ep.membership().epoch().get()).or_insert(false) |= ep.is_voter();
    let storage = &mut n.storage;
    match catch_unwind(AssertUnwindSafe(|| f(ep, storage, now))) {
      Ok(r) => {
        if let Some(ep) = n.ep.as_ref() {
          *n.voter_in.entry(ep.membership().epoch().get()).or_insert(false) |= ep.is_voter();
        }
        Some(r)
      }
      Err(p) => {
        let msg = panic_msg(p);
        self.panics.push((id, msg.clone()));
        self.crash_inner(id, Some(msg));
        None
      }
    }
  }

  /// Pump storage completions (+ block jobs) until the node's storage lane is quiet (storage is modelled
  /// as completing promptly), then collect every emitted message and event.
  pub fn pump(&mut self, id: u16) {
    for _ in 0..32 {
      self.pump_once(id);
      let quiet = match self.nodes[id as usize].ep.as_ref() {
        Some(ep) => !ep.has_inflight_storage(&self.nodes[id as usize].storage),
        None => true,
      };
      if quiet {
        break;
      }
    }
    self.collect(id);
  }

  fn pump_once(&mut self, id: u16) {
    loop {
      {
        let n = &mut self.nodes[id as usize];
        n.wal.borrow_mut().advance_device_clock(n.now);
      }
      if self.guarded(id, |ep, st, now| ep.handle_storage(now, st)).is_none() {
        return;
      }
      let job = {
        let n = &mut self.nodes[id as usize];
        n.storage.poll_block_job()
      };
      let Some(job) = job else { break };
      let done = {
        let n = &mut self.nodes[id as usize];
        execute_block_job(&mut n.lane, job, &mut n.blocks)
      };
      if self.guarded(id, |ep, st, now| ep.on_block_done(now, st, done)).is_none() {
        return;
      }
    }
  }

  fn collect(&mut self, id: u16) {
    let mut outs = Vec::new();
    let mut evs = Vec::new();
    let len = self.nodes.len() as u16;
    {
      let n = &mut self.nodes[id as usize];
      let Some(ep) = n.ep.as_mut() else { return };
      let voter = n.pre_voter || ep.is_voter();
      let epoch = ep.membership().epoch().get();
      let view = ep.view().get();
      let head = ep.op().get();
      while let Some(out) = ep.poll_message() {
        let to = out.to();
        // resolve a directed SLOT (in the sender's membership) to the target MemberId
        let targets: Vec<Option<u16>> = match to {
          Recipient::To(Peer::Replica(slot)) => {
            vec![Some(ep.member_at(slot).map_or(slot.get(), |m| m.get() as u16))]
          }
          Recipient::To(Peer::Client(_)) => vec![None],
          Recipient::To(_) => vec![],
          Recipient::Backups => (0..len).filter(|&x| x != id).map(Some).collect(),
          Recipient::AllReplicas => (0..len).map(Some).collect(),
        };
        let msg = out.into_msg();
        // the sender's role in the epoch the message is TAGGED with (votes carry their epoch)
        let (_, _, msg_epoch, _) = describe(&msg);
        let voter = match &msg {
          Message::PrepareOk(_) | Message::StartViewChange(_) | Message::DoViewChange(_) => {
            n.voter_in.get(&msg_epoch).copied().unwrap_or(voter)
          }
          _ => voter,
        };
        let tail_gap = matches!(&msg, Message::RequestPrepare(r) if r.op().get() > head);
        for t in targets {
          outs.push((t, msg.clone(), voter, epoch, view, tail_gap));
        }
      }
      while let Some(ev) = ep.poll_event() {
        evs.push(ev);
      }
      n.applied = ep.state_machine_ref().applied().to_vec();
      if ep.status() == Status::Retired {
        n.retired = true;
      }
    }
    for (t, msg, voter, epoch, view, tail_gap) in outs {
      match t {
        Some(to) => {
          self.seq += 1;
          self.net.push(Envelope {
            seq: self.seq,
            from: id,
            to,
            msg,
            delivered: false,
            from_was_voter: voter,
            from_epoch: epoch,
            from_view: view,
            aux: false,
            tail_gap,
          });
        }
        None => {
          if let Message::Reply(r) = &msg {
            self.replies.push((r.client().get(), r.request().get(), r.body_bytes()));
          }
        }
      }
    }
    for e in evs {
      self.events.push((id, e));
    }
  }

  /// Deliver envelope `k` (by index into `net`). Returns false if the recipient is down / unstarted.
  pub fn deliver(&mut self, k: usize) -> bool {
    self.net[k].delivered = true;
    let env = self.net[k].clone();
    let to = env.to;
    let n = &self.nodes[to as usize];
    if !n.up || n.ep.is_none() {
      return false;
    }
    // ingress translation: the sender's MemberId -> its slot in the RECEIVER's membership
    let from = {
      let ep = n.ep.as_ref().unwrap();
      match ep.slot_of(MemberId::new(env.from as u128)) {
        Some(slot) => Peer::Replica(slot),
        None => Peer::Replica(ReplicaId::new(env.from)),
      }
    };
    let msg = env.msg.clone();
    let serving_repair = matches!(msg, Message::RequestPrepare(_) | Message::RequestPrepareRange(_));
    let mark = self.net.len();
    if self
      .guarded(to, |ep, st, now| ep.handle_message(now, st, from, msg))
      .is_none()
    {
      return true;
    }
    self.pump(to);
    if serving_repair {
      for e in &mut self.net[mark..] {
        e.aux = true;
      }
    }
    true
  }

  /// Body repair is a sub-protocol BELOW the spec's abstraction (the spec's DVC/StartView carry
  /// bodies; viewstamp's carry headers and repair bodies peer-to-peer). Deliver pending repair traffic
  /// (requests, repair-serve responses, nacks) until none is left. A schedule restriction ("repair
  /// completes promptly"), never a fault-model extension.
  pub fn settle_repairs(&mut self, max: usize) -> usize {
    self.deliver_all_where(max, |e| {
      e.aux
        || matches!(
          e.msg,
          Message::RequestPrepare(_) | Message::RequestPrepareRange(_) | Message::RepairBatch(_)
            | Message::Nack(_)
        )
    })
  }

  /// Advance node `id`'s clock by `d` and service its timers.
  pub fn advance(&mut self, id: u16, d: Duration) {
    if !self.nodes[id as usize].up {
      return;
    }
    let n = &mut self.nodes[id as usize];
    n.now = n.now + d;
    if self
      .guarded(id, |ep, st, now| ep.handle_timeout(now, st))
      .is_none()
    {
      return;
    }
    self.pump(id);
  }

  /// Advance node `id`'s clock to its NEXT timer deadline and service it. Returns false if none armed.
  ///
  /// Mirrors viewstamp's production driver contract (viewstamp-reactor `service_consensus_timer`, the
  /// NONE-OR-DUE gate @ f7fbcd96): when NOTHING is armed, `handle_timeout` is serviced ONCE so a
  /// disarmed cadence (a natal / freshly-demoted learner's `learner_status`) self-arms; then the
  /// earliest armed deadline is fired as usual.
  pub fn fire_next_timer(&mut self, id: u16) -> bool {
    let up = self.nodes[id as usize].up;
    let none_armed = self.nodes[id as usize]
      .ep
      .as_ref()
      .is_some_and(|e| e.poll_timeout().is_none());
    if up && none_armed {
      if self
        .guarded(id, |ep, st, now| ep.handle_timeout(now, st))
        .is_none()
      {
        return true;
      }
      self.pump(id);
    }
    let Some(t) = self.nodes[id as usize]
      .ep
      .as_ref()
      .and_then(|e| e.poll_timeout())
    else {
      return false;
    };
    let n = &mut self.nodes[id as usize];
    if t.as_nanos() > n.now.as_nanos() {
      n.now = t;
    }
    if self
      .guarded(id, |ep, st, now| ep.handle_timeout(now, st))
      .is_none()
    {
      return true;
    }
    self.pump(id);
    true
  }

  fn crash_inner(&mut self, id: u16, why: Option<String>) {
    let n = &mut self.nodes[id as usize];
    if let Some(ep) = n.ep.as_ref() {
      n.applied = ep.state_machine_ref().applied().to_vec();
    }
    n.ep = None;
    n.up = false;
    if why.is_some() {
      n.panicked = why;
    }
    n.sb.borrow_mut().discard_inflight();
    n.wal.borrow_mut().discard_inflight();
    n.storage = Storage::new(n.wal.clone(), n.sb.clone());
  }

  pub fn crash(&mut self, id: u16) {
    self.crash_inner(id, None);
  }

  /// Restart over the node's own durable media (`recover_with_reconfig`).
  pub fn restart(&mut self, id: u16) -> Result<(), String> {
    let cfg = self.config(id);
    let genesis = self.genesis.clone();
    let n = &mut self.nodes[id as usize];
    n.incarnation += 1;
    let s = seed(id, n.incarnation);
    let rec = catch_unwind(AssertUnwindSafe(|| {
      Endpoint::<SimSm, SingleChange>::recover_with_reconfig(
        cfg,
        genesis,
        s,
        SimSm::Plain(LogSm::default()),
        &mut n.storage,
      )
    }));
    match rec {
      Ok(Ok(Recovered::Active(ep))) => {
        n.ep = Some(ep);
        n.up = true;
        n.panicked = None;
        self.pump(id);
        Ok(())
      }
      Ok(Ok(Recovered::Retired(_))) => {
        n.retired = true;
        n.up = true;
        Ok(())
      }
      Ok(Err(e)) => Err(format!("recover error: {e}")),
      Err(p) => {
        let m = panic_msg(p);
        self.panics.push((id, m.clone()));
        Err(format!("recover panicked: {m}"))
      }
    }
  }

  /// A client request with body = `label` (8 bytes BE), from a fresh client identity, delivered to `id`.
  pub fn client_request(&mut self, id: u16, label: u64) {
    let client = ClientId::new(1_000_000 + label as u128);
    let req = Request::new(client, RequestNumber::with(1), Bytes::from(label.to_be_bytes().to_vec()));
    if self
      .guarded(id, |ep, st, now| {
        ep.handle_message(now, st, Peer::Client(client), Message::Request(req))
      })
      .is_none()
    {
      return;
    }
    self.pump(id);
  }

  pub fn propose(
    &mut self,
    id: u16,
    delta: SingleVoterDelta,
    ack: bool,
  ) -> Option<Result<OpNumber, ProposeMembershipError>> {
    let ack = if ack { Some(AcceptReducedFaultTolerance) } else { None };
    let r = self.guarded(id, |ep, st, now| ep.propose_membership(now, st, delta, ack))?;
    self.pump(id);
    Some(r)
  }

  // ------------------------------------------------------------------------------------------------
  // Observation
  // ------------------------------------------------------------------------------------------------

  pub fn obs(&self, id: u16) -> Obs {
    let n = &self.nodes[id as usize];
    match n.ep.as_ref() {
      Some(ep) => {
        let (voters, learners) = member_ids(ep.membership());
        Obs {
          id,
          started: n.started,
          up: n.up,
          status: ep.status().as_str(),
          epoch: ep.membership().epoch().get(),
          voters,
          learners,
          view: ep.view().get(),
          log_view: ep.log_view().get(),
          op: ep.op().get(),
          commit: ep.commit().get(),
          commit_max: ep.commit_max().get(),
          is_primary: ep.is_primary() && ep.status().is_normal(),
        }
      }
      None => Obs {
        id,
        started: n.started,
        up: n.up,
        status: if n.retired { "retired" } else { "down" },
        epoch: 0,
        voters: vec![],
        learners: vec![],
        view: 0,
        log_view: 0,
        op: 0,
        commit: 0,
        commit_max: 0,
        is_primary: false,
      },
    }
  }

  /// Committed client labels in apply order (reconfiguration ops are not applied, so they are gaps).
  pub fn applied_labels(&self, id: u16) -> Vec<(u64, u64)> {
    self.nodes[id as usize]
      .applied
      .iter()
      .map(|(op, b)| {
        let mut x = [0u8; 8];
        let k = b.len().min(8);
        x[..k].copy_from_slice(&b[..k]);
        (*op, u64::from_be_bytes(x))
      })
      .collect()
  }

  /// Deliver every undelivered envelope (in emission order, including those emitted while draining)
  /// until the network is idle or `max` deliveries happened. Returns the number delivered.
  pub fn deliver_all(&mut self, max: usize) -> usize {
    let mut n = 0;
    while n < max {
      let Some(k) = (0..self.net.len()).find(|&k| !self.net[k].delivered) else { break };
      self.deliver(k);
      n += 1;
    }
    n
  }

  /// Like `deliver_all`, restricted to envelopes matching `pred` (others stay pending).
  pub fn deliver_all_where(&mut self, max: usize, pred: impl Fn(&Envelope) -> bool) -> usize {
    let mut n = 0;
    while n < max {
      let Some(k) = (0..self.net.len()).find(|&k| !self.net[k].delivered && pred(&self.net[k])) else {
        break;
      };
      self.deliver(k);
      n += 1;
    }
    n
  }

  /// Human-readable trace of envelopes `from..` (kind, from, to, view, epoch, op/commit).
  pub fn net_trace(&self, from: usize) -> String {
    let mut s = String::new();
    for e in &self.net[from..] {
      let (k, v, ep, op) = describe(&e.msg);
      s.push_str(&format!(
        "    #{} {} n{}->n{} v{} e{} op/commit={}{}\n",
        e.seq, k, e.from, e.to, v, ep, op, if e.delivered { " (delivered)" } else { "" }
      ));
    }
    s
  }

  /// Undelivered envelopes matching `pred`.
  pub fn pending(&self, pred: impl Fn(&Envelope) -> bool) -> Vec<usize> {
    (0..self.net.len())
      .filter(|&k| !self.net[k].delivered && pred(&self.net[k]))
      .collect()
  }

  /// Every envelope (delivered or not) matching `pred`.
  pub fn ever(&self, pred: impl Fn(&Envelope) -> bool) -> Vec<usize> {
    (0..self.net.len()).filter(|&k| pred(&self.net[k])).collect()
  }

  pub fn status_line(&self) -> String {
    let mut s = String::new();
    for id in 0..self.nodes.len() as u16 {
      let o = self.obs(id);
      if !o.started {
        continue;
      }
      if let (Some(ep), true) = (self.nodes[id as usize].ep.as_ref(), std::env::var("MBT_TRACE").is_ok()) {
        s.push_str(&format!(
          "  n{id} [internals] log_len={} inflight={} deferred_appends={} storage_inflight={} install={:?} status={}\n",
          ep.log_len(),
          ep.inflight_len(),
          ep.deferred_appends_len(),
          ep.has_inflight_storage(&self.nodes[id as usize].storage),
          ep.debug_install_state(),
          ep.status().as_str()
        ));
        // durable WAL slots (header view / client / request) and the checkpoint op
        let wal = self.nodes[id as usize].wal.clone();
        let slots: Vec<String> = (1..=ep.op().get().max(8))
          .filter_map(|op| {
            viewstamp_proto::Wal::header(&wal, OpNumber::with(op))
              .map(|h| format!("{op}:v{}/c{}/r{}", h.view().get(), h.client().get(), h.request().get()))
          })
          .collect();
        s.push_str(&format!("  n{id} [wal] checkpoint={} slots={slots:?}\n", ep.checkpoint_op().get()));
      }
      s.push_str(&format!(
        "  n{} {}{} e{} V{:?} L{:?} v{} lv{} op{} commit{} cmax{}{} applied={:?}\n",
        id,
        if o.up { "" } else { "DOWN " },
        o.status,
        o.epoch,
        o.voters,
        o.learners,
        o.view,
        o.log_view,
        o.op,
        o.commit,
        o.commit_max,
        if o.is_primary { " PRIMARY" } else { "" },
        self.applied_labels(id)
      ));
    }
    s
  }
}

/// A short, stable description of a message kind + the fields the spec keys deliveries on.
pub fn describe(m: &Message) -> (&'static str, u64, u64, u64) {
  // (kind, view, epoch, op-or-commit)
  match m {
    Message::Prepare(p) => ("Prepare", p.view().get(), p.epoch().get(), p.op().get()),
    Message::PrepareOk(p) => ("PrepareOk", p.view().get(), p.epoch().get(), p.op().get()),
    Message::Commit(c) => ("Commit", c.view().get(), c.epoch().get(), c.commit().get()),
    Message::StartViewChange(s) => ("StartViewChange", s.view().get(), s.epoch().get(), 0),
    Message::DoViewChange(d) => ("DoViewChange", d.view().get(), d.epoch().get(), d.op().get()),
    Message::StartView(s) => ("StartView", s.view().get(), s.epoch().get(), s.op().get()),
    Message::LearnerStatus(l) => ("LearnerStatus", 0, l.epoch().get(), 0),
    other => (other.kind_str(), 0, 0, 0),
  }
}
