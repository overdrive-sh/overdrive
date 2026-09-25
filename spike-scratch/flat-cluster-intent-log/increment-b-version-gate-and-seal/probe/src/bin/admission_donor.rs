//! Spike probe (increment B, resume): what an APPLICATION-LEVEL halt leaks on viewstamp's current API.
//!
//! Same `GateSm` as `src/main.rs` (shared source via `#[path]`), same loopback-TCP reactor stream
//! driver. Genesis: voters 0,1,2 (binary v2), learner 3 (binary v1), learner 4 (binary v2, started late
//! on an EMPTY, unformatted store).
//!
//! Phase 1 — the gate commits v2; learner 3 halts at the gate op INSIDE its state machine, while the
//!           viewstamp endpoint under it keeps advancing `commit_min` and checkpointing.
//! Phase D — poisoned donor: with the halted learner 3 still RUNNING, a fresh learner 4 joins on an empty
//!           store T times; each time it state-syncs from whichever member answers. The halted learner's
//!           checkpoints carry its FROZEN image under later op numbers. Per trial: does learner 4 end up
//!           with the voters' state (IDENTICAL) or the frozen image (DIVERGED)?
//! Phase A — admission: promote the halted v1 learner 3 to VOTER through viewstamp's own reconfiguration
//!           API (`Handle::reconfigure_to`) after the cluster committed v2, then make its acknowledgement
//!           load-bearing (stop voter 2: quorum 3 of {0,1,3}) and show the commit needs it (stop 3 too:
//!           no commit).
//!
//! Output is plain text; the harness prints its own observations as `OBSERVE ...` / `CHECK ...`.

#[path = "../gate_sm.rs"]
mod gate_sm;

use std::{
  collections::{BTreeMap, BTreeSet, HashMap},
  net::SocketAddr,
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};

use agnostic::tokio::TokioRuntime;
use bytes::Bytes;
use gate_sm::GateSm;
use viewstamp_proto::{
  BlockAddress, BlockStore, ClientId, Config, Conn, Epoch, Event, LabelOptions, Labeled, MemberId,
  Membership, MembershipTarget, Passthrough, Peer, ReplicaId, Superblock, Wal,
};
use viewstamp_reactor::{BlockLane, Handle};
use viewstamp_simulation::{InMemorySuperblock, InMemoryWal};

const CLUSTER: u128 = 0x1B_6A7E_0003;
const NODES: u16 = 5; // slots 0,1,2 voters; 3,4 learners
const HALTED: u16 = 3; // learner, binary v1
const LATE: u16 = 4; // learner, binary v2, joins late on an empty store
const CHECKPOINT_OPS: u64 = 4;
const BASE_PORT: u16 = 48131;
const TRIALS: u32 = 12;

fn genesis() -> Membership {
  Membership::from_durable_parts(Epoch::new(0), 3, 2, (0..u128::from(NODES)).map(MemberId::new).collect(), 0)
    .expect("valid genesis membership")
}

// ------------------------------------------------------------------ storage devices (as in main.rs)

struct Dev<T> {
  inner: Arc<Mutex<T>>,
  ready: flume::Sender<()>,
}
impl<T> Dev<T> {
  fn new(inner: Arc<Mutex<T>>, ready: flume::Sender<()>) -> Self {
    Self { inner, ready }
  }
  fn signal(&self) {
    let _ = self.ready.try_send(());
  }
}
impl<T: Wal> Wal for Dev<T> {
  fn op_head(&self) -> viewstamp_proto::OpNumber {
    self.inner.lock().unwrap().op_head()
  }
  fn header(&self, op: viewstamp_proto::OpNumber) -> Option<viewstamp_proto::Header> {
    self.inner.lock().unwrap().header(op)
  }
  fn status(&self, op: viewstamp_proto::OpNumber) -> viewstamp_proto::SlotStatus {
    self.inner.lock().unwrap().status(op)
  }
  fn capacity(&self) -> u64 {
    self.inner.lock().unwrap().capacity()
  }
  fn submit_append(
    &mut self,
    id: viewstamp_proto::WriteId,
    op: viewstamp_proto::OpNumber,
    header: viewstamp_proto::Header,
    body: Bytes,
  ) {
    self.inner.lock().unwrap().submit_append(id, op, header, body);
    self.signal();
  }
  fn submit_read(&mut self, id: viewstamp_proto::ReadId, op: viewstamp_proto::OpNumber) {
    self.inner.lock().unwrap().submit_read(id, op);
    self.signal();
  }
  fn truncate(&mut self, above: viewstamp_proto::OpNumber) -> Vec<viewstamp_proto::WriteId> {
    self.inner.lock().unwrap().truncate(above)
  }
  fn prune(&mut self, below: viewstamp_proto::OpNumber) -> Vec<viewstamp_proto::WriteId> {
    self.inner.lock().unwrap().prune(below)
  }
  fn poll(&mut self) -> Option<viewstamp_proto::WalDone> {
    self.inner.lock().unwrap().poll()
  }
}
impl<T: Superblock> Superblock for Dev<T> {
  fn state(&self) -> viewstamp_proto::VsrState {
    self.inner.lock().unwrap().state()
  }
  fn submit_write(&mut self, id: viewstamp_proto::WriteId, state: viewstamp_proto::VsrState) {
    self.inner.lock().unwrap().submit_write(id, state);
    self.signal();
  }
  fn submit_write_checkpoint(&mut self, id: viewstamp_proto::WriteId, op: viewstamp_proto::OpNumber, snapshot: Bytes) {
    self.inner.lock().unwrap().submit_write_checkpoint(id, op, snapshot);
    self.signal();
  }
  fn submit_read_checkpoint(&mut self, id: viewstamp_proto::ReadId) {
    self.inner.lock().unwrap().submit_read_checkpoint(id);
    self.signal();
  }
  fn poll(&mut self) -> Option<viewstamp_proto::SuperblockDone> {
    self.inner.lock().unwrap().poll()
  }
}

#[derive(Clone, Default)]
struct Blocks(Arc<Mutex<HashMap<BlockAddress, Bytes>>>);
impl BlockStore for Blocks {
  fn read_block(&self, addr: BlockAddress) -> Option<Bytes> {
    self.0.lock().unwrap().get(&addr).cloned()
  }
  fn put(&mut self, block: Bytes) -> BlockAddress {
    let addr = viewstamp_proto::block_address(&block);
    self.0.lock().unwrap().insert(addr, block);
    addr
  }
  fn flush(&mut self) -> Result<(), viewstamp_proto::BlockStoreError> {
    Ok(())
  }
  fn has_block(&self, addr: BlockAddress) -> bool {
    self.0.lock().unwrap().contains_key(&addr)
  }
}

#[derive(Clone)]
struct Media {
  wal: Arc<Mutex<InMemoryWal>>,
  sb: Arc<Mutex<InMemorySuperblock>>,
  blocks: Blocks,
}
impl Media {
  fn fresh() -> Self {
    Self {
      wal: Arc::new(Mutex::new(InMemoryWal::new())),
      sb: Arc::new(Mutex::new(InMemorySuperblock::new())),
      blocks: Blocks::default(),
    }
  }
}

// ------------------------------------------------------------------ event recording

#[derive(Clone, Debug)]
struct Rec {
  incarnation: String,
  kind: &'static str,
  op: u64,
  text: String,
}

type Log = Arc<Mutex<BTreeMap<u16, Vec<Rec>>>>;

fn spawn_recorder(node: u16, incarnation: &str, handle: &Handle, log: Log) {
  let rx = handle.events();
  let incarnation = incarnation.to_owned();
  drop(tokio::spawn(async move {
    while let Ok(ev) = rx.recv_async().await {
      let (kind, op, text) = match ev {
        Event::Committed(c) => ("committed", c.op().get(), String::from_utf8_lossy(c.reply()).into_owned()),
        Event::CheckpointDurable(op) => ("checkpoint", op.get(), String::new()),
        Event::StateSyncStarted(op) => ("state-sync-started", op.get(), String::new()),
        Event::StateSyncCompleted(op) => ("state-sync", op.get(), String::new()),
        Event::StatusChanged(s) => ("status", 0, format!("{s:?}")),
        Event::MembershipChanged(m) => (
          "membership",
          m.op().get(),
          format!(
            "epoch={:?} config_id={:x} self_is_voter={} self_is_learner={}",
            m.epoch(),
            m.config_id(),
            m.self_is_voter(),
            m.self_is_learner()
          ),
        ),
        _ => continue,
      };
      log.lock().unwrap().entry(node).or_default().push(Rec {
        incarnation: incarnation.clone(),
        kind,
        op,
        text,
      });
    }
  }));
}

fn addr(i: u16) -> SocketAddr {
  format!("127.0.0.1:{}", BASE_PORT + i).parse().unwrap()
}

async fn start_node(id: u16, supported_max: u16, media: &Media, client: u128, format: bool) -> Handle {
  let peers: Vec<(ReplicaId, SocketAddr)> =
    (0..NODES).filter(|&p| p != id).map(|p| (ReplicaId::new(p), addr(p))).collect();
  let config = Config::try_new(CLUSTER, MemberId::new(u128::from(id)))
    .and_then(|c| c.with_checkpoint_interval(CHECKPOINT_OPS))
    .expect("config");
  let (ready_tx, ready_rx) = flume::unbounded();
  let wal = Dev::new(media.wal.clone(), ready_tx.clone());
  let mut sb = Dev::new(media.sb.clone(), ready_tx);
  if format {
    viewstamp_driver::format(config, &genesis(), &wal, &mut sb).expect("format");
  }
  let opts = LabelOptions::new(CLUSTER, Peer::Replica(ReplicaId::new(id)));
  let mk_dialer: Arc<dyn Fn(Peer) -> Conn<Labeled<Passthrough>> + Send + Sync> =
    Arc::new(move |_peer| Conn::from_parts(Labeled::dialer(Passthrough::new(), &opts)));
  let mk_acceptor: Arc<dyn Fn() -> Conn<Labeled<Passthrough>> + Send + Sync> =
    Arc::new(move || Conn::from_parts(Labeled::acceptor(Passthrough::new(), &opts)));
  let blocks = BlockLane::spawn(media.blocks.clone());
  let (driver, handle) = viewstamp_reactor::ReactorStreamDriver::<TokioRuntime, _, _, _, _>::new(
    config,
    genesis(),
    GateSm::new(supported_max),
    wal,
    sb,
    blocks,
    ClientId::new(client),
    0,
    addr(id),
    peers,
    mk_dialer,
    mk_acceptor,
    ready_rx,
  )
  .await
  .expect("driver builds");
  drop(tokio::spawn(driver.run()));
  handle
}

async fn try_submit(h: &Handle, body: Bytes, within: Duration) -> Option<String> {
  let deadline = Instant::now() + within;
  while Instant::now() < deadline {
    let left = deadline.saturating_duration_since(Instant::now());
    match tokio::time::timeout(left.min(Duration::from_secs(10)), h.submit(body.clone())).await {
      Ok(Ok(reply)) => return Some(String::from_utf8_lossy(&reply).into_owned()),
      Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(100)).await,
      Err(_) => {}
    }
  }
  None
}

async fn submit(h: &Handle, body: Bytes, label: &str) -> String {
  try_submit(h, body, Duration::from_secs(120))
    .await
    .unwrap_or_else(|| panic!("submit {label} never committed"))
}

fn committed(log: &Log, node: u16, incarnation: &str) -> BTreeMap<u64, String> {
  log
    .lock()
    .unwrap()
    .get(&node)
    .map(|v| {
      v.iter()
        .filter(|r| r.kind == "committed" && r.incarnation == incarnation)
        .map(|r| (r.op, r.text.clone()))
        .collect()
    })
    .unwrap_or_default()
}

fn events(log: &Log, node: u16, incarnation: &str, kind: &str) -> Vec<Rec> {
  log
    .lock()
    .unwrap()
    .get(&node)
    .map(|v| v.iter().filter(|r| r.kind == kind && r.incarnation == incarnation).cloned().collect())
    .unwrap_or_default()
}

async fn await_op(log: &Log, node: u16, incarnation: &str, op: u64, within: Duration) -> bool {
  let deadline = Instant::now() + within;
  loop {
    if committed(log, node, incarnation).contains_key(&op) {
      return true;
    }
    if Instant::now() > deadline {
      return false;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
  text.split_whitespace().find_map(|w| w.strip_prefix(key))
}

fn verdict(text: &str) -> &str {
  text.split_whitespace().next().unwrap_or("")
}

fn op_of(reply: &str) -> u64 {
  field(reply, "op=").and_then(|s| s.parse().ok()).unwrap_or(0)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
  let t0 = Instant::now();
  println!("viewstamp rev f7fbcd96ebcee2aafa08f3ac136844743c7f9773 — admission + poisoned-donor probe");
  println!("genesis: voters 0,1,2 (binary v2); learner 3 (binary v1); learner 4 (binary v2, joins late, empty store)");
  println!("checkpoint interval: {CHECKPOINT_OPS} ops; trials: {TRIALS}\n");

  let log: Log = Arc::new(Mutex::new(BTreeMap::new()));
  let media: Vec<Media> = (0..LATE).map(|_| Media::fresh()).collect();
  let mut handles = Vec::new();
  for id in 0..LATE {
    let supported = if id == HALTED { 1 } else { 2 };
    let h = start_node(id, supported, &media[id as usize], u128::from(id) + 1, true).await;
    spawn_recorder(id, if id == HALTED { "learner3-v1" } else { "voter-v2" }, &h, log.clone());
    handles.push(h);
  }
  let inc = |id: u16| if id == HALTED { "learner3-v1" } else { "voter-v2" };

  // ---------------------------------------------------------------- phase 1: the gate
  println!("== phase 1: gate v2 commits; learner 3 (v1) halts inside its state machine");
  let mut plan: Vec<(u16, &str, Bytes)> = vec![(0, "op alpha (v1)", gate_sm::op(1, b"alpha"))];
  for id in 0..3u16 {
    plan.push((id, "advertise voter (v2)", gate_sm::advertise(1, u128::from(id), 2)));
  }
  plan.push((1, "advertise learner 3 (v1), via voter 1", gate_sm::advertise(1, 3, 1)));
  plan.push((2, "advertise learner 4 (v2), via voter 2", gate_sm::advertise(1, 4, 2)));
  plan.push((0, "gate v2 over voters {0,1,2}", gate_sm::gate(1, 2, &[0, 1, 2])));
  for n in 0..9 {
    plan.push((0, "op filler (v2)", gate_sm::op(2, format!("filler-{n}").as_bytes())));
  }
  let mut gate_op = 0;
  for (via, label, body) in &plan {
    let reply = submit(&handles[*via as usize], body.clone(), label).await;
    if verdict(&reply) == "GATE-COMMITTED" {
      gate_op = op_of(&reply);
    }
    println!("  via slot {via}: {label:<40} -> {reply}");
  }
  let last = op_of(&committed(&log, 0, "voter-v2").values().last().cloned().unwrap_or_default());
  for id in 0..LATE {
    assert!(await_op(&log, id, inc(id), last, Duration::from_secs(30)).await, "slot {id} reached op {last}");
  }
  tokio::time::sleep(Duration::from_millis(500)).await;
  let halted_reply = committed(&log, HALTED, "learner3-v1").get(&gate_op).cloned().unwrap_or_default();
  println!("  learner 3 at gate op {gate_op}: {halted_reply}");
  for id in 0..LATE {
    let cps: Vec<u64> = events(&log, id, inc(id), "checkpoint").iter().map(|r| r.op).collect();
    println!("  slot {id} CheckpointDurable ops: {cps:?}");
  }
  let halted_cps_after: Vec<u64> =
    events(&log, HALTED, "learner3-v1", "checkpoint").iter().map(|r| r.op).filter(|op| *op > gate_op).collect();
  println!(
    "OBSERVE halted learner 3 made checkpoints durable ABOVE its halt op {gate_op}: {halted_cps_after:?} \
     (its SM image is frozen at op {})",
    gate_op - 1
  );

  // ---------------------------------------------------------------- phase D: poisoned donor
  println!("\n== phase D: learner 3 stays up (halted); a fresh v2 learner 4 joins on an EMPTY store, {TRIALS} times");
  let mut identical = 0u32;
  let mut diverged = 0u32;
  let mut other = 0u32;
  for trial in 0..TRIALS {
    let label = format!("learner4-trial{trial:02}");
    let fresh = Media::fresh();
    let h4 = start_node(LATE, 2, &fresh, 3_000 + u128::from(trial), false).await;
    spawn_recorder(LATE, &label, &h4, log.clone());
    let r = submit(&handles[0], gate_sm::op(2, format!("trial-{trial}").as_bytes()), "trial op").await;
    let x = op_of(&r);
    let caught = await_op(&log, LATE, &label, x, Duration::from_secs(30)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mine = committed(&log, LATE, &label).get(&x).cloned();
    let syncs: Vec<u64> = events(&log, LATE, &label, "state-sync").iter().map(|r| r.op).collect();
    let d4 = mine.as_deref().and_then(|t| field(t, "digest="));
    let dv = field(&r, "digest=");
    let outcome = match (caught, d4, dv) {
      (true, Some(a), Some(b)) if a == b => {
        identical += 1;
        "IDENTICAL"
      }
      (true, Some(_), Some(_)) => {
        diverged += 1;
        "DIVERGED"
      }
      _ => {
        other += 1;
        "NO-COMPARABLE-REPLY"
      }
    };
    println!("  trial {trial:02}: state-syncs at {syncs:?}; op {x}: {outcome}");
    println!("      learner 4: {}", mine.as_deref().unwrap_or("<none>"));
    println!("      voter 0  : {r}");
    let _ = h4.shutdown().await;
  }
  println!(
    "OBSERVE poisoned donor: {identical} IDENTICAL, {diverged} DIVERGED, {other} no comparable reply, out of {TRIALS} trials"
  );

  // ---------------------------------------------------------------- phase A: admission
  println!("\n== phase A: promote the HALTED v1 learner 3 to voter via Handle::reconfigure_to (cluster version is v2)");
  let target = MembershipTarget::new(
    BTreeSet::from([0u128, 1, 2, 3].map(MemberId::new)),
    BTreeSet::from([MemberId::new(4)]),
  );
  let before_ops: Vec<u64> = committed(&log, 0, "voter-v2").keys().copied().collect();
  let res = tokio::time::timeout(
    Duration::from_secs(60),
    handles[0].reconfigure_to(target, viewstamp_driver::HealthHint::default(), None),
  )
  .await;
  println!("  reconfigure_to -> {res:?}");
  tokio::time::sleep(Duration::from_millis(500)).await;
  for id in 0..LATE {
    for m in events(&log, id, inc(id), "membership") {
      println!("  slot {id} MembershipChanged at op {}: {}", m.op, m.text);
    }
  }
  let r = submit(&handles[0], gate_sm::op(2, b"after-promotion"), "op after promotion").await;
  println!("  via slot 0: op after promotion -> {r}");
  let x = op_of(&r);
  let prev = before_ops.last().copied().unwrap_or(0);
  let skipped: Vec<u64> = ((prev + 1)..x).filter(|op| !committed(&log, 0, "voter-v2").contains_key(op)).collect();
  println!(
    "OBSERVE voter 0's apply stream jumps from op {prev} to op {x}; op(s) {skipped:?} never reached StateMachine::apply \
     (the Reconfigure op is consumed by the endpoint)"
  );

  println!("\n== phase A2: stop voter 2 -> voters {{0,1,3}} live, quorum 3 of 4: every commit now needs halted voter 3's ack");
  let rep = handles[2].shutdown().await;
  println!("  voter 2 shutdown: {rep:?}");
  let r = try_submit(&handles[0], gate_sm::op(2, b"needs-voter-3"), Duration::from_secs(60)).await;
  println!("  via slot 0: op needing voter 3 -> {r:?}");
  if let Some(r) = &r {
    let x = op_of(r);
    let ok = await_op(&log, HALTED, "learner3-v1", x, Duration::from_secs(10)).await;
    println!(
      "  halted voter 3's own apply output at op {x} (caught={ok}): {:?}",
      committed(&log, HALTED, "learner3-v1").get(&x)
    );
  }
  println!("\n== phase A3: also stop voter 3 -> only {{0,1}} live: a commit must now be impossible");
  let rep = handles[HALTED as usize].shutdown().await;
  println!("  voter 3 shutdown: {rep:?}");
  let r2 = try_submit(&handles[0], gate_sm::op(2, b"no-quorum"), Duration::from_secs(8)).await;
  println!("  via slot 0: op with only {{0,1}} live (8 s budget) -> {r2:?}");
  println!(
    "OBSERVE halted-voter ack load-bearing: commit with {{0,1,3}} = {}, commit with {{0,1}} = {}",
    r.is_some(),
    r2.is_some()
  );

  for id in [0u16, 1] {
    let _ = handles[id as usize].shutdown().await;
  }
  println!("\nelapsed: {} ms", t0.elapsed().as_millis());
}
