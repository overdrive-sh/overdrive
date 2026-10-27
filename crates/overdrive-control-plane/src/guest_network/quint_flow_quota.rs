//! Quint conformance driver — `flow_quota`, conformance main `quota_conf`:
//! the per-allocation flow quota counts half-closed flows, and the host
//! releases a flow's place when it reads the flow's end record (ADR-0208,
//! ADR-0209 / EVMOD-AB; QPROJ-1, QPROJ-3, QPROJ-8; § *Per-module projection
//! and action map* "`flow_quota`"; D-UP-1).
//!
//! Replays `specs/quint/guest-flow-owner/flow_quota.qnt` (referenced in
//! place) on its recording main `quota_conf` — the configuration of
//! `quota_ok`: three allocations with quota Q = 1 over a node of two flow
//! places — against the production owner over `SimGuestFlowIo` and its
//! `SimFlowForwarder` arbiter. The step record is read through
//! `Config::nondet = &["quota_conf::flow_quota::conf_step"]`; the state is
//! projected by qualified names ([`Qualified`]).
//!
//! **Realising Q = 1 and CAP = 2 against the production bounds.** The quota
//! is the pinned constant `GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION` (4,096) and
//! the node's flow places `GUEST_FLOW_MAX_PAIRS` (65,536). At `init` the
//! guest of each model allocation opens 4,095 background flows, and further
//! background allocations open flows until the node holds 65,534 — every
//! background flow admitted, read, connected, attached and paired by the
//! production owner, never ended, outside the model (stimulus). Each model
//! allocation then stands one flow below its quota and the node two places
//! below its capacity: a model flow is the last place of its allocation and
//! one of the node's last two. The background is fixed for the trace.
//!
//! | Action | Production call or fault |
//! |---|---|
//! | `init` | a fresh composition boots; the leases, sessions and the background flows are opened through the owner's driving ports |
//! | `Admit(a)` | the guest of `a` opens a TcpConnect flow; the seam delivers it; the owner admits, reads, connects, attaches the pair and sends `Paired` |
//! | `Refuse(a)` | the same stimulus; the model expects `Refused(Capacity)`: the flow is closed, nothing else changes. The driver records production's verdict (held or not), so a flow the owner admits instead occupies a place in the projection |
//! | `HalfClose(s)` | the destination of the place's flow sends its FIN; the forwarder carries it after the last accepted byte |
//! | `Complete(s)` | the guest half-closes its direction too: the pair completes and queues its record |
//! | `Abort(s)` | the destination resets: the pair aborts and queues its record |
//! | `ReadEnd(s)` | the seam releases the next queued end record to the owner's dispatch (`take_end` returns the head of the context's queue, M-1), which releases the flow's place (EVMOD-AB); the model enables `ReadEnd(s)` only for the place at the head of `endq`, so the released record must be place `s`'s |
//!
//! Projection: `slot(s).own` ← O the allocation of the CID whose
//! `guest_flows` holds the place's flow (the place ↔ flow bijection is the
//! driver's, fixed at `Admit` by the model's lowest-free-place rule; 0 when
//! no held flow); `slot(s).st` ← O the flow's phase (`Paired { pair,
//! generation }`) with S the forwarder's facts (`ended`: the pair's id is in
//! `queued_ends` of the context row at the pair's generation — its record
//! queued and not yet read; `active`: `pair_state` shows no direction's
//! source ended; `half`: one direction's source ended clean with its target
//! shut; `free`: no held flow); `endq` ← S the same row's `queued_ends`, in
//! queue order, relabelled to places (M-1: the order `take_end` reads them;
//! D-12, the named arbiter whose Tier-3 pair is the module suite's
//! `take_end_returns_records_in_end_order`).
//! `slot(s).fwd` is model-only (the `clash` ghost); `g` is ghost;
//! `conf_step` model-only.
//!
//! Every named run of `quota_conf` is pinned with a `#[quint_test]`, the two
//! M-1 queue-order runs (`EndRecordsAreReadInQueueOrder`,
//! `EndRecordsAreReleasedInQueueOrder`) included. Sources: the owner is held
//! directly (`HostSharedGuestNetworkOwner`, QPROJ-3's `flow_observation`); the
//! forwarder facts are `SimFlowForwarder`'s, the named arbiter (QPROJ-7 /
//! QPROJ-8).

#![allow(
    clippy::needless_pass_by_ref_mut,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::print_stderr,
    reason = "a conformance driver asserts its own preconditions and prints its seed; protocol names are prose"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use itf::Value;
use overdrive_core::guest_transport::forwarder::{ForwardId, ForwarderGeneration, SourceEnd};
use overdrive_core::guest_transport::wire::{FlowId, GUEST_FLOW_MAX_PAIRS};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::owner_acceptance::support::{Composition, Lease, paused_runtime};
use super::quint_itf::{self, ItfError, Qualified};
use super::{
    GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, GuestConnKey, GuestFlowPhase, HostSharedGuestNetworkOwner,
};

const MAIN: &str = "quota_conf";
const MODULE: &str = "flow_quota";
/// The model's `ALLOCS` and `SLOTS`.
const ALLOCS: [i64; 3] = [1, 2, 3];
const SLOTS: [i64; 2] = [1, 2];
/// Background flows of each model allocation: one below the quota.
const MODEL_BACKGROUND: u32 = GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION - 1;
/// The node's flow places left free for the model: its `CAP`.
const MODEL_CAP: u32 = 2;
/// Flows the driver opens between settles, bounding the sim's accept queue.
const BATCH: u32 = 64;
const SEED: u64 = 0x0F70;

/// `slot(s)`: `(own, st)`; `endq`: the places whose end records are
/// queued, in queue order (M-1).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct QuotaState {
    slot: BTreeMap<i64, (i64, String)>,
    endq: Vec<i64>,
}

impl QuotaState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let q = Qualified::of(value, MAIN, MODULE)?;
        let slot = quint_itf::int_map(q.var("slot")?, |slot| {
            Ok((
                quint_itf::int(quint_itf::field(slot, "own")?)?,
                quint_itf::string(quint_itf::field(slot, "st")?)?,
            ))
        })?;
        let endq = quint_itf::list_elems(q.var("endq")?)?
            .into_iter()
            .map(quint_itf::int)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self { slot, endq })
    }
}

impl<'de> Deserialize<'de> for QuotaState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<QuotaDriver> for QuotaState {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &QuotaDriver) -> Result<Self> {
        driver.project()
    }
}

/// One model flow at its place: its connection and its flow id (the
/// bijection; never a source of state).
#[derive(Debug, Clone, Copy)]
struct PlaceFlow {
    key: GuestConnKey,
    flow: FlowId,
}

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` cannot be played: {why}")]
    NotPlayable { action: &'static str, why: String },
}

fn not_playable(action: &'static str, why: impl Into<String>) -> anyhow::Error {
    DriverError::NotPlayable { action, why: why.into() }.into()
}

pub(super) struct QuotaDriver {
    rt: tokio::runtime::Runtime,
    /// The production owner under test (QPROJ-3), the composition's own.
    owner: Arc<HostSharedGuestNetworkOwner>,
    /// The composition the owner stands on (its seam sim and pool).
    c: Composition,
    /// Model allocation ↦ its lease (and its live session).
    leases: BTreeMap<i64, Lease>,
    /// Model place ↦ the model flow occupying it.
    places: BTreeMap<i64, PlaceFlow>,
    next_seq: u32,
    next_port: u32,
}

impl QuotaDriver {
    fn new() -> Self {
        let rt = paused_runtime();
        let c = {
            let _runtime = rt.enter();
            Composition::non_mesh(SEED)
        };
        Self {
            rt,
            owner: Arc::clone(&c.owner),
            c,
            leases: BTreeMap::new(),
            places: BTreeMap::new(),
            next_seq: 1,
            next_port: 10_000,
        }
    }

    fn composition(&self) -> &Composition {
        &self.c
    }

    fn port(&mut self) -> u32 {
        self.next_port += 1;
        self.next_port
    }

    fn seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Opens `count` paired background flows for `lease`, settling every
    /// [`BATCH`] flows.
    fn background(&mut self, lease: &Lease, count: u32) {
        let mut opened = 0;
        while opened < count {
            for _ in 0..BATCH.min(count - opened) {
                let (seq, port) = (self.seq(), self.port());
                let destination = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 20), 443);
                let _background = self.composition().open_tcp_flow(lease, seq, destination, port);
                opened += 1;
            }
            self.rt.block_on(self.composition().settle());
        }
    }

    fn init(&mut self) -> Result {
        *self = Self::new();
        self.rt.block_on(self.c.boot());
        // `ReadEnd` is the trace's: the owner reads an end record only when
        // the seam releases it.
        self.c.io.set_auto_ends(false);
        for a in ALLOCS {
            let (lease, _session) =
                self.rt.block_on(self.composition().active_lease(&format!("quota-{a}"), &[]));
            self.background(&lease, MODEL_BACKGROUND);
            self.leases.insert(a, lease);
        }
        let model_background = u32::try_from(ALLOCS.len()).unwrap_or(0) * MODEL_BACKGROUND;
        let mut filler = GUEST_FLOW_MAX_PAIRS - MODEL_CAP - model_background;
        let mut index = 0;
        while filler > 0 {
            index += 1;
            let (lease, _session) = self
                .rt
                .block_on(self.composition().active_lease(&format!("quota-filler-{index}"), &[]));
            let count = filler.min(GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION);
            self.background(&lease, count);
            filler -= count;
        }
        self.rt.block_on(self.composition().settle());
        let held: u32 = self
            .rt
            .block_on(self.owner.flow_observation())
            .cids
            .values()
            .map(|cid| cid.held_flows)
            .sum();
        if held != GUEST_FLOW_MAX_PAIRS - MODEL_CAP {
            anyhow::bail!(
                "the background holds {held} flows, not {}",
                GUEST_FLOW_MAX_PAIRS - MODEL_CAP
            );
        }
        Ok(())
    }

    /// The guest of model allocation `a` opens one TcpConnect flow, which the
    /// owner admits or refuses. The driver records production's verdict, not
    /// the action's expectation: a flow the owner holds after the settle takes
    /// the lowest free place (the model's `minFree`), a flow it refused takes
    /// none. A wrongly admitted `Refuse`, or a wrongly refused `Admit`, then
    /// shows in the projected `slot` and `endq` and diverges from the spec.
    fn open_model_flow(&mut self, a: i64, action: &'static str) -> Result {
        let lease = self
            .leases
            .get(&a)
            .ok_or_else(|| not_playable(action, format!("no allocation {a}")))?
            .clone();
        let (seq, port) = (self.seq(), self.port());
        let destination = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 30), 443);
        let (key, flow) = self.composition().open_tcp_flow(&lease, seq, destination, port);
        self.rt.block_on(self.composition().settle());
        let observed = self.rt.block_on(self.owner.flow_observation());
        let admitted = observed.cids.values().any(|cid| cid.guest_flows.contains_key(&key));
        if admitted {
            let place =
                SLOTS.iter().copied().find(|s| !self.places.contains_key(s)).ok_or_else(|| {
                    not_playable(
                        action,
                        "the owner admitted a flow with no free place in the driver's bijection",
                    )
                })?;
            self.places.insert(place, PlaceFlow { key, flow });
        }
        Ok(())
    }

    fn place(&self, s: i64, action: &'static str) -> Result<PlaceFlow> {
        self.places
            .get(&s)
            .copied()
            .ok_or_else(|| not_playable(action, format!("place {s} holds no flow")))
    }

    fn half_close(&mut self, s: i64) -> Result {
        let flow = self.place(s, "HalfClose")?.flow;
        self.composition().io.destination_fins(flow);
        self.rt.block_on(self.composition().settle());
        Ok(())
    }

    fn complete(&mut self, s: i64) -> Result {
        let flow = self.place(s, "Complete")?.flow;
        self.composition().io.guest_half_closes(flow);
        self.rt.block_on(self.composition().settle());
        Ok(())
    }

    fn abort(&mut self, s: i64) -> Result {
        let flow = self.place(s, "Abort")?.flow;
        self.composition().io.destination_resets(flow);
        self.rt.block_on(self.composition().settle());
        Ok(())
    }

    /// `ReadEnd(s)`: the seam releases one end record; the owner's dispatch
    /// takes the head of the context's queue (M-1) and releases its flow's
    /// place. The model enables `ReadEnd(s)` only when place `s` is at the
    /// head of `endq`, so the flow that left the owner's table must be `s`'s;
    /// any other outcome is a divergence of the production read order.
    fn read_end(&mut self, s: i64) -> Result {
        let place = self.place(s, "ReadEnd")?;
        self.c.io.release_end();
        self.rt.block_on(self.c.settle());
        let observed = self.rt.block_on(self.owner.flow_observation());
        let still_held = observed.cids.values().any(|cid| cid.guest_flows.contains_key(&place.key));
        if still_held {
            anyhow::bail!(
                "the released end record was not place {s}'s: the owner reads end records in their queue order (take_end, M-1)"
            );
        }
        self.places.remove(&s);
        Ok(())
    }

    fn project(&self) -> Result<QuotaState> {
        let mut slot: BTreeMap<i64, (i64, String)> =
            SLOTS.iter().map(|s| (*s, (0, "free".to_owned()))).collect();
        let observed = self.rt.block_on(self.owner.flow_observation());
        // conformance-lint: sim-arbiter-ok QPROJ-8 / D-12 SimFlowForwarder context table (queued_ends: the end records queued and not yet read, in queue order), Tier-3 pairs V-EV-CTL and take_end_returns_records_in_end_order (the module suite through KernelFlowForwarder)
        let contexts = self.c.io.forwarder_contexts();
        // The device retains phase 1's released probe context, while the
        // owner's `forwarder_generation` counts only serving contexts. Keep
        // the probe row visible and validate its full complement before
        // projecting a serving generation over the exact table suffix.
        let serving_count = usize::try_from(observed.forwarder_generation.get())?;
        let phase_count = contexts.len().checked_sub(serving_count).ok_or_else(|| {
            anyhow::anyhow!(
                "owner generation {serving_count} exceeds the device's {} context rows",
                contexts.len()
            )
        })?;
        if phase_count != 1 {
            anyhow::bail!(
                "the device has {phase_count} non-serving context rows; phase 1 opens exactly one"
            );
        }
        let phase = &contexts[0];
        if phase.open
            || phase.live_pairs != 0
            || phase.live_watches != 0
            || !phase.queued_ends.is_empty()
        {
            anyhow::bail!("phase 1's released probe context retained resources: {phase:?}");
        }
        let serving_contexts = &contexts[phase_count..];
        let mut place_of: BTreeMap<(ForwarderGeneration, ForwardId), i64> = BTreeMap::new();
        for (s, place) in &self.places {
            let holder =
                observed.cids.values().find(|cid| cid.guest_flows.contains_key(&place.key));
            let Some(cid) = holder else { continue };
            let model_alloc = self
                .leases
                .iter()
                .find(|(_, lease)| lease.alloc == cid.alloc)
                .map(|(a, _)| *a)
                .ok_or_else(|| {
                    anyhow::anyhow!("place {s}'s flow is held by a non-model allocation")
                })?;
            let Some(GuestFlowPhase::Paired { pair, generation }) =
                cid.guest_flows.get(&place.key).copied()
            else {
                anyhow::bail!("place {s}'s flow is held but not Paired");
            };
            place_of.insert((generation, pair), *s);
            let row = usize::try_from(generation.get())
                .ok()
                .and_then(|g| g.checked_sub(1))
                .and_then(|i| serving_contexts.get(i))
                .ok_or_else(|| anyhow::anyhow!("place {s}'s pair names no context row"))?;
            let st = if row.queued_ends.contains(&pair) {
                "ended"
            } else {
                // conformance-lint: sim-arbiter-ok QPROJ-8 SimFlowForwarder pair_state (per-direction source end and target shut), Tier-3 pair V-EV-END and the module suite through KernelFlowForwarder
                match self.c.io.forwarder_pair_state(pair) {
                    None => anyhow::bail!(
                        "place {s}'s pair is neither live nor queued: its record was produced and lost"
                    ),
                    Some(state) => {
                        let ended = [state.a_to_b, state.b_to_a]
                            .into_iter()
                            .filter(|direction| {
                                direction.source_end == Some(SourceEnd::Clean)
                                    && direction.target_shut
                            })
                            .count();
                        if ended == 0 { "active" } else { "half" }
                    }
                }
            };
            slot.insert(*s, (model_alloc, st.to_owned()));
        }
        // M-1: the queue the owner's `take_end` reads, head first. Only the
        // model's flows end (the background never does), so every queued id
        // has a place.
        let mut endq = Vec::new();
        for (index, row) in serving_contexts.iter().enumerate() {
            let generation = ForwarderGeneration::new(u64::try_from(index + 1)?)?;
            for id in &row.queued_ends {
                let place = place_of.get(&(generation, *id)).ok_or_else(|| {
                    anyhow::anyhow!("a queued end record of {id:?} has no model place")
                })?;
                endq.push(*place);
            }
        }
        Ok(QuotaState { slot, endq })
    }
}

impl Driver for QuotaDriver {
    type State = QuotaState;

    fn config() -> Config {
        Config { state: &[], nondet: &["quota_conf::flow_quota::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init()?,
            Admit(a: i64) => self.open_model_flow(a, "Admit")?,
            Refuse(a: i64) => self.open_model_flow(a, "Refuse")?,
            HalfClose(s: i64) => self.half_close(s)?,
            Complete(s: i64) => self.complete(s)?,
            Abort(s: i64) => self.abort(s)?,
            ReadEnd(s: i64) => self.read_end(s)?,
        })
    }
}

// ----- named runs (quota_conf, max_samples = 1) and randomized traces ------

/// Named run `AdmissionStopsAtTheAllocationQuota` (`quota_conf`).
/// CONTRACT_SHAPE: unbounded-preservation.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "AdmissionStopsAtTheAllocationQuota"
)]
fn quint_flow_quota_admission_stops_at_the_allocation_quota() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `DrainingFlowStillCountsAgainstTheQuota` (`quota_conf`).
/// CONTRACT_SHAPE: unbounded-preservation.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "DrainingFlowStillCountsAgainstTheQuota"
)]
fn quint_flow_quota_draining_flow_still_counts_against_the_quota() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `WithholdingGuestHoldsAtMostItsOwnQuota` (`quota_conf`).
/// CONTRACT_SHAPE: unbounded-preservation.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "WithholdingGuestHoldsAtMostItsOwnQuota"
)]
fn quint_flow_quota_withholding_guest_holds_at_most_its_own_quota() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `CompletedPairReleasesTheSlotAtItsEndRecord` (`quota_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "CompletedPairReleasesTheSlotAtItsEndRecord"
)]
fn quint_flow_quota_completed_pair_releases_the_slot_at_its_end_record() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `AbortOfHalfClosedFlowReleasesTheSlotAtItsEndRecord`
/// (`quota_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "AbortOfHalfClosedFlowReleasesTheSlotAtItsEndRecord"
)]
fn quint_flow_quota_abort_of_half_closed_flow_releases_the_slot_at_its_end_record() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `EndRecordsAreReadInQueueOrder` (`quota_conf`; M-1): two pairs
/// end, place 2's first; the projected `endq` is `[2, 1]` (the order
/// `take_end` reads the context's records). The run's closing
/// `ReadEnd(1)` is not enabled in the model (`.fail()`), so the replayed
/// trace ends at the second `Abort`; the queue order itself is the compared
/// fact.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "EndRecordsAreReadInQueueOrder"
)]
fn quint_flow_quota_end_records_are_read_in_queue_order() -> impl Driver {
    QuotaDriver::new()
}

/// Named run `EndRecordsAreReleasedInQueueOrder` (`quota_conf`; M-1): two
/// pairs end, place 2's first; the owner's first read releases place 2 and
/// leaves place 1 ended with its record queued, the second releases place 1
/// (`read_end` fails the step if the released record is not the head's).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 1,
    test = "EndRecordsAreReleasedInQueueOrder"
)]
fn quint_flow_quota_end_records_are_released_in_queue_order() -> impl Driver {
    QuotaDriver::new()
}

/// Randomized `quota_conf` traces; the seed is printed on failure. Each
/// trace builds the node's 65,534-flow background, so the sample count is
/// bounded.
#[quint_run(
    spec = "../../specs/quint/guest-flow-owner/flow_quota.qnt",
    main = "quota_conf",
    max_samples = 8
)]
fn quint_flow_quota_randomized_traces() -> impl Driver {
    QuotaDriver::new()
}
