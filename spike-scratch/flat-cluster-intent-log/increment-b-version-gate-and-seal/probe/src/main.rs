//! Spike probe (increment B): an application-level committed cluster-version gate on viewstamp.
//!
//! A real 4-replica viewstamp cluster over loopback TCP (the reactor stream driver on tokio,
//! `Labeled<Passthrough>` handshake — the shape of viewstamp's own `three_node` example):
//! three voters whose binary supports v2 and one learner whose binary supports only v1.
//!
//! Phase 1 — the gate: ops, per-node `Advertise` facts, a gate the rule must REJECT (v3), the gate
//! that commits (v2), then v2-format ops. Every replica's own `Event::Committed` reply stream is
//! recorded: that stream is the replica's `StateMachine::apply` output, so it is the ground truth of
//! what each replica applied.
//! Phase 2 — process-level halt: the halted learner's driver is shut down; the voters keep committing.
//! Phase 3 — upgrade of the halted learner to a v2 binary, two ways:
//!   3a restart IN PLACE over its own store (naive resume: un-halt once the binary supports v2);
//!   3b a fresh empty store under the same member id (wipe + rejoin as a learner).
//!
//! Output is plain text; the harness prints its own checks as `CHECK ... PASS|FAIL`.

mod gate_sm;

use std::{
  collections::{BTreeMap, HashMap},
  net::SocketAddr,
  sync::{Arc, Mutex},
  time::{Duration, Instant},
};

use agnostic::tokio::TokioRuntime;
use bytes::Bytes;
use gate_sm::GateSm;
use viewstamp_proto::{
  BlockAddress, BlockStore, ClientId, Config, Conn, Epoch, Event, LabelOptions, Labeled, MemberId,
  Membership, Passthrough, Peer, ReplicaId, Superblock, Wal,
};
use viewstamp_reactor::{BlockLane, Handle};
use viewstamp_simulation::{InMemorySuperblock, InMemoryWal};

const CLUSTER: u128 = 0x1B_6A7E_0002;
const NODES: u16 = 4; // slots 0,1,2 voters; slot 3 learner
const LEARNER: u16 = 3;
const CHECKPOINT_OPS: u64 = 4;
const BASE_PORT: u16 = 47931;

/// Genesis: 3 voters + 1 learner, MemberId i at slot i (config_id 0, like viewstamp's examples).
fn genesis() -> Membership {
  Membership::from_durable_parts(Epoch::new(0), 3, 1, (0..u128::from(NODES)).map(MemberId::new).collect(), 0)
    .expect("valid genesis membership")
}

// ------------------------------------------------------------------ storage devices (Send, shared)

/// A durable medium the harness keeps a handle to, so a restarted driver can reopen it. Signals the
/// driver's storage-ready notifier on every submit (the in-memory fixtures complete synchronously).
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

/// Content-addressed block store for SM checkpoints, shared so a restart can reopen it.
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

/// One replica's durable media.
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
  incarnation: &'static str,
  kind: &'static str,
  op: u64,
  text: String,
}

type Log = Arc<Mutex<BTreeMap<u16, Vec<Rec>>>>;

fn spawn_recorder(node: u16, incarnation: &'static str, handle: &Handle, log: Log) {
  let rx = handle.events();
  drop(tokio::spawn(async move {
    while let Ok(ev) = rx.recv_async().await {
      let rec = match ev {
        Event::Committed(c) => Some(Rec {
          incarnation,
          kind: "committed",
          op: c.op().get(),
          text: String::from_utf8_lossy(c.reply()).into_owned(),
        }),
        Event::CheckpointDurable(op) => Some(Rec {
          incarnation,
          kind: "checkpoint",
          op: op.get(),
          text: String::new(),
        }),
        Event::StateSyncCompleted(op) => Some(Rec {
          incarnation,
          kind: "state-sync",
          op: op.get(),
          text: String::new(),
        }),
        Event::StatusChanged(s) => Some(Rec {
          incarnation,
          kind: "status",
          op: 0,
          text: format!("{s:?}"),
        }),
        _ => None,
      };
      if let Some(r) = rec {
        log.lock().unwrap().entry(node).or_default().push(r);
      }
    }
  }));
}

// ------------------------------------------------------------------ cluster plumbing

fn addr(i: u16) -> SocketAddr {
  format!("127.0.0.1:{}", BASE_PORT + i).parse().unwrap()
}

async fn try_start_node(
  id: u16,
  supported_max: u16,
  media: &Media,
  client: u128,
  format: bool,
) -> Result<Handle, String> {
  let peers: Vec<(ReplicaId, SocketAddr)> =
    (0..NODES).filter(|&p| p != id).map(|p| (ReplicaId::new(p), addr(p))).collect();
  let config = Config::try_new(CLUSTER, MemberId::new(u128::from(id)))
    .and_then(|c| c.with_checkpoint_interval(CHECKPOINT_OPS))
    .expect("config");
  let (ready_tx, ready_rx) = flume::unbounded();
  let wal = Dev::new(media.wal.clone(), ready_tx.clone());
  let mut sb = Dev::new(media.sb.clone(), ready_tx);
  if format {
    // One-time cluster-creation step (a restarting member never formats).
    viewstamp_driver::format(config, &genesis(), &wal, &mut sb).map_err(|e| format!("format: {e:?}"))?;
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
  .map_err(|e| format!("driver build: {e:?}"))?;
  drop(tokio::spawn(driver.run()));
  Ok(handle)
}

async fn start_node(id: u16, supported_max: u16, media: &Media, client: u128, format: bool) -> Handle {
  try_start_node(id, supported_max, media, client, format).await.expect("node starts")
}

async fn submit(h: &Handle, body: Bytes, label: &str) -> String {
  for attempt in 0..30 {
    match tokio::time::timeout(Duration::from_secs(10), h.submit(body.clone())).await {
      Ok(Ok(reply)) => return String::from_utf8_lossy(&reply).into_owned(),
      Ok(Err(e)) => {
        if attempt % 20 == 0 {
          println!("  (submit {label}: {e:?}, retrying)");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
      Err(_) => println!("  (submit {label}: timed out after 10s, retrying)"),
    }
  }
  panic!("submit {label} never committed");
}

/// Wait until `node` has recorded a `committed` event for every op in `ops` (from `incarnation`).
async fn await_ops(log: &Log, node: u16, incarnation: &str, ops: &[u64], within: Duration) -> bool {
  let deadline = Instant::now() + within;
  loop {
    let have: Vec<u64> = log
      .lock()
      .unwrap()
      .get(&node)
      .map(|v| v.iter().filter(|r| r.kind == "committed" && r.incarnation == incarnation).map(|r| r.op).collect())
      .unwrap_or_default();
    if ops.iter().all(|o| have.contains(o)) {
      return true;
    }
    if Instant::now() > deadline {
      return false;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
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

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
  text.split_whitespace().find_map(|w| w.strip_prefix(key))
}

fn verdict(text: &str) -> &str {
  text.split_whitespace().next().unwrap_or("")
}

fn check(name: &str, ok: bool, detail: String) -> bool {
  println!("CHECK {name}: {} — {detail}", if ok { "PASS" } else { "FAIL" });
  ok
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
  let t0 = Instant::now();
  println!("viewstamp rev f7fbcd96ebcee2aafa08f3ac136844743c7f9773 — gate probe");
  println!("cluster: slots 0,1,2 = voters (binary supports v2); slot 3 = learner (binary supports v1)");
  println!("checkpoint interval: {CHECKPOINT_OPS} ops\n");

  let log: Log = Arc::new(Mutex::new(BTreeMap::new()));
  let media: Vec<Media> = (0..NODES).map(|_| Media::fresh()).collect();
  let mut handles = Vec::new();
  for id in 0..NODES {
    let supported = if id == LEARNER { 1 } else { 2 };
    let h = start_node(id, supported, &media[id as usize], u128::from(id) + 1, true).await;
    spawn_recorder(id, if id == LEARNER { "learner-v1" } else { "voter-v2" }, &h, log.clone());
    handles.push(h);
  }

  // ---------------------------------------------------------------- phase 1: the gate
  println!("== phase 1: submit sequence (reply shown is the SUBMITTING replica's apply output)");
  let mut plan: Vec<(u16, &str, Bytes)> = vec![
    (0, "op alpha (tag v1)", gate_sm::op(1, b"alpha")),
    (0, "op bravo (tag v1)", gate_sm::op(1, b"bravo")),
  ];
  for id in 0..LEARNER {
    plan.push((id, "advertise (voter, own handle)", gate_sm::advertise(1, u128::from(id), 2)));
  }
  // A learner is not a client-ingress member in viewstamp: its fact reaches the log via a voter.
  plan.push((1, "advertise (learner slot 3's fact, via voter 1)", gate_sm::advertise(1, u128::from(LEARNER), 1)));
  plan.push((0, "gate v3 over voters {0,1,2} — must be REJECTED", gate_sm::gate(1, 3, &[0, 1, 2])));
  plan.push((0, "gate v2 over voters {0,1,2} — must COMMIT", gate_sm::gate(1, 2, &[0, 1, 2])));
  plan.push((0, "op charlie (tag v2)", gate_sm::op(2, b"charlie")));
  plan.push((0, "op delta (tag v1, N-1 format)", gate_sm::op(1, b"delta")));
  plan.push((0, "op echo (tag v3, uncommitted format)", gate_sm::op(3, b"echo")));
  for n in 0..5 {
    plan.push((0, "op filler (tag v2)", gate_sm::op(2, format!("filler-{n}").as_bytes())));
  }
  println!("  one bounded attempt through the LEARNER's own handle (Handle::submit, 5 s):");
  match tokio::time::timeout(Duration::from_secs(5), handles[LEARNER as usize].submit(gate_sm::op(1, b"from-learner"))).await {
    Ok(Ok(r)) => println!("    learner submit committed: {}", String::from_utf8_lossy(&r)),
    Ok(Err(e)) => println!("    learner submit returned error: {e:?}"),
    Err(_) => println!("    learner submit did NOT resolve within 5 s (no error returned)"),
  }
  let mut last_op = 0u64;
  for (via, label, body) in &plan {
    let reply = submit(&handles[*via as usize], body.clone(), label).await;
    let op: u64 = field(&reply, "op=").and_then(|s| s.parse().ok()).unwrap_or(0);
    last_op = last_op.max(op);
    println!("  via slot {via}: {label:<48} -> {reply}");
  }
  let all_ops: Vec<u64> = committed(&log, 0, "voter-v2").keys().copied().collect();
  let mut reached = true;
  for id in 0..NODES {
    let inc = if id == LEARNER { "learner-v1" } else { "voter-v2" };
    reached &= await_ops(&log, id, inc, &all_ops, Duration::from_secs(30)).await;
  }
  tokio::time::sleep(Duration::from_millis(500)).await;

  println!("\n== phase 1: every replica's own apply stream (Event::Committed replies), by op");
  let streams: Vec<BTreeMap<u64, String>> = (0..NODES)
    .map(|id| committed(&log, id, if id == LEARNER { "learner-v1" } else { "voter-v2" }))
    .collect();
  let ops: Vec<u64> = streams[0].keys().copied().collect();
  for op in &ops {
    println!("  op {op:>2}");
    for id in 0..NODES {
      println!("    slot {id}: {}", streams[id as usize].get(op).map(String::as_str).unwrap_or("<none>"));
    }
  }
  println!("\n== phase 1: checkpoint events (CheckpointDurable op) per replica");
  for id in 0..NODES {
    let cps: Vec<u64> = log
      .lock()
      .unwrap()
      .get(&id)
      .map(|v| v.iter().filter(|r| r.kind == "checkpoint").map(|r| r.op).collect())
      .unwrap_or_default();
    println!("  slot {id}: {cps:?}");
  }

  // ---------------------------------------------------------------- phase 1 checks
  println!("\n== phase 1 checks");
  let mut all = true;
  all &= check("all four replicas emitted a commit for every op", reached, format!("{} ops", ops.len()));
  let voters_agree = ops.iter().all(|op| {
    let a = streams[0].get(op);
    a.is_some() && streams[1].get(op) == a && streams[2].get(op) == a
  });
  all &= check("voters' apply outputs are identical at every op", voters_agree, String::new());
  let gate_op = ops.iter().copied().find(|op| verdict(&streams[0][op]) == "GATE-COMMITTED");
  let rejected_op = ops.iter().copied().find(|op| verdict(&streams[0][op]) == "GATE-REJECTED");
  all &= check(
    "gate v3 rejected on every replica (not every voter advertised v3)",
    rejected_op.is_some_and(|op| (0..NODES).all(|id| verdict(&streams[id as usize][&op]) == "GATE-REJECTED")),
    format!("op {rejected_op:?}"),
  );
  all &= check("gate v2 committed on the voters", gate_op.is_some(), format!("op {gate_op:?}"));
  let gate_op = gate_op.unwrap_or(u64::MAX);
  let learner = &streams[LEARNER as usize];
  let prefix_identical = ops.iter().filter(|op| **op < gate_op).all(|op| learner.get(op) == streams[0].get(op));
  all &= check(
    "learner's apply output is identical to the voters' for every op BEFORE the gate",
    prefix_identical,
    String::new(),
  );
  all &= check(
    "learner HALTs at the gate op (does not apply it)",
    learner.get(&gate_op).is_some_and(|t| verdict(t) == "HALT"),
    learner.get(&gate_op).cloned().unwrap_or_default(),
  );
  let after: Vec<&u64> = ops.iter().filter(|op| **op > gate_op).collect();
  all &= check(
    "learner applies NOTHING after the gate (every later op is SKIP-HALTED)",
    !after.is_empty() && after.iter().all(|op| learner.get(op).is_some_and(|t| verdict(t) == "SKIP-HALTED")),
    format!("{} later ops", after.len()),
  );
  let pre_gate_voter_digest = ops
    .iter()
    .filter(|op| **op < gate_op)
    .last()
    .and_then(|op| field(&streams[0][op], "digest="))
    .map(str::to_owned);
  let learner_frozen_digests: std::collections::BTreeSet<&str> = ops
    .iter()
    .filter(|op| **op >= gate_op)
    .filter_map(|op| learner.get(op).and_then(|t| field(t, "digest=")))
    .collect();
  all &= check(
    "learner's frozen state digest == voters' digest just before the gate (a prefix, not a divergence)",
    learner_frozen_digests.len() == 1
      && pre_gate_voter_digest.as_deref() == learner_frozen_digests.iter().next().copied(),
    format!("voters@gate-1={pre_gate_voter_digest:?} learner-after-gate={learner_frozen_digests:?}"),
  );
  let continued = after.iter().any(|op| verdict(&streams[0][op]) == "APPLIED");
  all &= check("voters (v2) continue applying v2-format entries after the gate", continued, String::new());
  let learner_cps_after: Vec<u64> = log
    .lock()
    .unwrap()
    .get(&LEARNER)
    .map(|v| v.iter().filter(|r| r.kind == "checkpoint" && r.op > gate_op).map(|r| r.op).collect())
    .unwrap_or_default();
  println!(
    "OBSERVE learner endpoint made checkpoints DURABLE at ops above its halt point {gate_op}: {learner_cps_after:?} \
     (each binds the HALTED image — state as of op {} — to a later op number)",
    gate_op.saturating_sub(1)
  );

  // ---------------------------------------------------------------- phase 2: process-level halt
  println!("\n== phase 2: the embedder stops the halted learner's process; voters keep committing");
  let report = handles[LEARNER as usize].shutdown().await;
  println!("  learner shutdown: {report:?}");
  let r = submit(&handles[1], gate_sm::op(2, b"after-learner-stopped"), "op after learner stopped").await;
  println!("  via slot 1: op after learner stopped -> {r}");
  all &= check("cluster commits with the learner stopped", verdict(&r) == "APPLIED", r.clone());
  let final_ops: Vec<u64> = committed(&log, 0, "voter-v2").keys().copied().collect();

  // ---------------------------------------------------------------- phase 3a: restart in place
  println!("\n== phase 3a: upgrade the learner to a v2 binary and restart it IN PLACE over its own store");
  let h3a = start_node(LEARNER, 2, &media[LEARNER as usize], 1_000 + u128::from(LEARNER), false).await;
  spawn_recorder(LEARNER, "learner-v2-inplace", &h3a, log.clone());
  let caught = await_ops(&log, LEARNER, "learner-v2-inplace", &[*final_ops.last().unwrap()], Duration::from_secs(20)).await;
  tokio::time::sleep(Duration::from_millis(500)).await;
  let s3a = committed(&log, LEARNER, "learner-v2-inplace");
  println!("  restarted learner emitted {} commits (ops {:?}..{:?}); caught up to last op: {caught}", s3a.len(), s3a.keys().next(), s3a.keys().last());
  for (op, t) in &s3a {
    println!("    op {op:>2}: {t}");
    println!("       voter: {}", streams[0].get(op).map(String::as_str).unwrap_or("<phase-2 op>"));
  }
  let voter_last = committed(&log, 0, "voter-v2");
  let last = *final_ops.last().unwrap();
  let inplace_last = s3a.get(&last).and_then(|t| field(t, "digest=")).map(str::to_owned);
  let voter_last_digest = voter_last.get(&last).and_then(|t| field(t, "digest=")).map(str::to_owned);
  println!(
    "OBSERVE in-place restart: learner digest at op {last} = {inplace_last:?}; voters = {voter_last_digest:?} -> {}",
    match (&inplace_last, &voter_last_digest) {
      (Some(a), Some(b)) if a == b => "IDENTICAL",
      (Some(_), Some(_)) => "DIVERGED",
      _ => "no comparable reply",
    }
  );
  let _ = h3a.shutdown().await;

  // ---------------------------------------------------------------- phase 3b: wipe + rejoin
  println!("\n== phase 3b: upgrade the learner to a v2 binary on a FRESH EMPTY store (wipe + rejoin as learner)");
  let fresh = Media::fresh();
  let h3b = match try_start_node(LEARNER, 2, &fresh, 2_000 + u128::from(LEARNER), false).await {
    Ok(h) => {
      println!("  wiped learner started on an UNFORMATTED empty store");
      h
    }
    Err(e) => {
      println!("  unformatted empty store refused ({e}); formatting it with the genesis membership instead");
      let fresh = Media::fresh();
      start_node(LEARNER, 2, &fresh, 2_000 + u128::from(LEARNER), true).await
    }
  };
  spawn_recorder(LEARNER, "learner-v2-fresh", &h3b, log.clone());
  // Drive one more op so a lagging learner has a reason to sync.
  let r = submit(&handles[0], gate_sm::op(2, b"after-rejoin"), "op after rejoin").await;
  println!("  via slot 0: op after rejoin -> {r}");
  let last2: u64 = field(&r, "op=").and_then(|s| s.parse().ok()).unwrap_or(0);
  let caught = await_ops(&log, LEARNER, "learner-v2-fresh", &[last2], Duration::from_secs(30)).await;
  tokio::time::sleep(Duration::from_millis(500)).await;
  let s3b = committed(&log, LEARNER, "learner-v2-fresh");
  let syncs: Vec<u64> = log
    .lock()
    .unwrap()
    .get(&LEARNER)
    .map(|v| v.iter().filter(|x| x.kind == "state-sync" && x.incarnation == "learner-v2-fresh").map(|x| x.op).collect())
    .unwrap_or_default();
  println!("  fresh learner: {} commits, state-syncs at {syncs:?}, caught up to op {last2}: {caught}", s3b.len());
  let v_after = committed(&log, 0, "voter-v2");
  let fresh_d = s3b.get(&last2).and_then(|t| field(t, "digest=")).map(str::to_owned);
  let voter_d = v_after.get(&last2).and_then(|t| field(t, "digest=")).map(str::to_owned);
  println!("  fresh learner op {last2}: {:?}", s3b.get(&last2));
  println!("  voter         op {last2}: {:?}", v_after.get(&last2));
  println!(
    "OBSERVE wipe+rejoin: learner digest at op {last2} = {fresh_d:?}; voters = {voter_d:?} -> {}",
    match (&fresh_d, &voter_d) {
      (Some(a), Some(b)) if a == b => "IDENTICAL",
      (Some(_), Some(_)) => "DIVERGED",
      _ => "no comparable reply",
    }
  );

  for h in handles.iter().take(3) {
    let _ = h.shutdown().await;
  }
  let _ = h3b.shutdown().await;
  println!("\nPHASE-1/2 RESULT: {}", if all { "ALL CHECKS PASS" } else { "SOME CHECKS FAILED" });
  println!("elapsed: {} ms", t0.elapsed().as_millis());
}
