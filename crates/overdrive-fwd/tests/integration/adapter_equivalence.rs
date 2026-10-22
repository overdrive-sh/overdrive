//! R5-57, the real-adapter half (D-UP-17; PD-5): the same stimulus
//! sequence driven against `KernelFlowForwarder` and against
//! `SimFlowForwarder` — the named arbiter of the forwarder's kernel facts
//! (QPROJ-8) — yields the same inventory, the same live pair states and the
//! same end records after every step. This is what makes a seeded run of
//! the owners over `SimFlowForwarder` evidence about the real forwarder.
//!
//! The production-owner half of R5-57 (N flows held through the host owner
//! on the real adapter) belongs to the composed suites.
//!
//! The sim rig drives `SimFlowForwarder` through the port and through the
//! sim's scripting surface (`SimForwarderDevice::open`, `SimSocket`'s
//! `peer_shutdown` / `peer_reset`, `SimFlowForwarder::run_until_idle`); the
//! sim runs no work item behind the test's back, so the rig settles it by
//! running every queued run before each observation, as the kernel rig
//! settles by waiting.

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::thread;

use overdrive_core::guest_transport::forwarder::{
    FlowForwarder, ForwardId, ForwardOutcome, ForwarderInventory, PairMode, PairState,
};
use overdrive_fwd::KernelFlowForwarder;
use overdrive_sim::adapters::{SimFlowForwarder, SimForwarderDevice, SimShutdown, SimSocket};

use super::support::{Vsock, open_context, reset_close, tcp_pair, vsock_pair};

/// How long the kernel rig lets the forwarder's work items run before it
/// reads the port (the measured propagation is well under a millisecond
/// typical, P-68).
const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);

/// One stimulus. Indices name the item (pair or watch) created by the
/// `Attach` / `Watch` step with that index.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// A host TcpConnect pair: half A a vsock, half B a TCP socket.
    Attach(usize),
    /// A watched dial `V_h`.
    Watch(usize),
    /// The item's vsock far end half-closes (`0x2`).
    GuestHalfClose(usize),
    /// The item's vsock far end closes with its direction open (`0x3`).
    GuestClose(usize),
    /// The pair's TCP far end half-closes (FIN).
    DestinationFin(usize),
    /// The pair's TCP far end resets.
    DestinationReset(usize),
    /// The owner aborts the item.
    OwnerAbort(usize),
    /// The owner aborts everything.
    AbortAll,
}

/// What both adapters must agree on after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observation {
    inventory: ForwarderInventory,
    /// Every end record taken since the previous step, by item index.
    ends: BTreeMap<usize, ForwardOutcome>,
    /// The state of every live pair, by item index.
    pairs: BTreeMap<usize, PairState>,
}

trait Rig {
    fn apply(&mut self, step: Step);
    fn observe(&mut self) -> Observation;
}

/// Collects records and live pair states over the port's own accessors.
fn observe_port<S: ?Sized>(
    fwd: &dyn FlowForwarder<S>,
    ids: &BTreeMap<usize, ForwardId>,
    pairs: &[usize],
    settle: impl Fn(),
) -> Observation {
    let mut ends = BTreeMap::new();
    loop {
        settle();
        let mut any = false;
        while let Some(end) = fwd.take_end().expect("take_end") {
            let index = ids
                .iter()
                .find_map(|(i, id)| (*id == end.id).then_some(*i))
                .expect("a record names an id this run created");
            assert!(ends.insert(index, end.outcome).is_none(), "one record per id");
            any = true;
        }
        if !any {
            break;
        }
    }
    let pairs = pairs
        .iter()
        .filter_map(|i| fwd.pair_state(ids[i]).expect("pair_state").map(|s| (*i, s)))
        .collect();
    Observation { inventory: fwd.inventory().expect("inventory"), ends, pairs }
}

// ---------------------------------------------------------------------------
// The kernel rig: real sockets on the host role
// ---------------------------------------------------------------------------

#[derive(Default)]
struct KernelItem {
    _v: Option<Vsock>,
    v_far: Option<Vsock>,
    _t: Option<std::net::TcpStream>,
    t_far: Option<std::net::TcpStream>,
}

struct KernelRig {
    fwd: KernelFlowForwarder,
    ids: BTreeMap<usize, ForwardId>,
    items: BTreeMap<usize, KernelItem>,
    pairs: Vec<usize>,
}

impl KernelRig {
    fn new() -> Self {
        Self {
            fwd: open_context(),
            ids: BTreeMap::new(),
            items: BTreeMap::new(),
            pairs: Vec::new(),
        }
    }
}

impl Rig for KernelRig {
    fn apply(&mut self, step: Step) {
        match step {
            Step::Attach(i) => {
                let (v, v_far) = vsock_pair();
                let (t, t_far) = tcp_pair();
                let id = self
                    .fwd
                    .attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream)
                    .expect("attach");
                self.ids.insert(i, id);
                self.pairs.push(i);
                self.items.insert(
                    i,
                    KernelItem { _v: Some(v), v_far: Some(v_far), _t: Some(t), t_far: Some(t_far) },
                );
            }
            Step::Watch(i) => {
                let (v, v_far) = vsock_pair();
                let id = self.fwd.watch(&v as &dyn AsFd).expect("watch");
                self.ids.insert(i, id);
                self.items.insert(
                    i,
                    KernelItem { _v: Some(v), v_far: Some(v_far), ..KernelItem::default() },
                );
            }
            Step::GuestHalfClose(i) => {
                self.items[&i].v_far.as_ref().expect("guest end").shutdown(nix::libc::SHUT_WR);
            }
            Step::GuestClose(i) => drop(self.items.get_mut(&i).expect("item").v_far.take()),
            Step::DestinationFin(i) => {
                let t_far = self.items[&i].t_far.as_ref().expect("destination");
                t_far.shutdown(std::net::Shutdown::Write).expect("FIN");
            }
            Step::DestinationReset(i) => {
                reset_close(
                    self.items.get_mut(&i).expect("item").t_far.take().expect("destination"),
                );
            }
            Step::OwnerAbort(i) => {
                self.fwd.abort(self.ids[&i]).expect("abort");
            }
            Step::AbortAll => {
                self.fwd.abort_all().expect("abort_all");
            }
        }
    }

    fn observe(&mut self) -> Observation {
        observe_port(&self.fwd, &self.ids, &self.pairs, || thread::sleep(SETTLE))
    }
}

// ---------------------------------------------------------------------------
// The sim rig
// ---------------------------------------------------------------------------

/// The same stimuli against `SimFlowForwarder`: each sim socket is the
/// half the owner holds, and its far side is scripted (`peer_*`).
struct SimRig {
    /// Keeps the context table alive for the rig's life.
    _device: SimForwarderDevice,
    fwd: SimFlowForwarder,
    ids: BTreeMap<usize, ForwardId>,
    /// The vsock half and, for a pair, the TCP half.
    sockets: BTreeMap<usize, (SimSocket, Option<SimSocket>)>,
    pairs: Vec<usize>,
}

impl SimRig {
    fn new() -> Self {
        let device = SimForwarderDevice::new();
        let fwd = device.open();
        Self {
            _device: device,
            fwd,
            ids: BTreeMap::new(),
            sockets: BTreeMap::new(),
            pairs: Vec::new(),
        }
    }

    fn vsock(&self, i: usize) -> &SimSocket {
        &self.sockets[&i].0
    }

    fn destination(&self, i: usize) -> &SimSocket {
        self.sockets[&i].1.as_ref().expect("a pair has a destination half")
    }
}

impl Rig for SimRig {
    fn apply(&mut self, step: Step) {
        match step {
            Step::Attach(i) => {
                let v = SimSocket::vsock_stream();
                let t = SimSocket::tcp();
                let id = self.fwd.attach(&v, &t, PairMode::Stream).expect("attach");
                self.ids.insert(i, id);
                self.pairs.push(i);
                self.sockets.insert(i, (v, Some(t)));
            }
            Step::Watch(i) => {
                let v = SimSocket::vsock_stream();
                let id = self.fwd.watch(&v).expect("watch");
                self.ids.insert(i, id);
                self.sockets.insert(i, (v, None));
            }
            // The kernel rig's `shutdown(SHUT_WR)` of the far end: `0x2`.
            Step::GuestHalfClose(i) => self.vsock(i).peer_shutdown(SimShutdown::Send),
            // The kernel rig's `close()` of the far end with its direction
            // open: `0x3` (`virtio_transport_common.c:1288-1295 @ v7.0`).
            Step::GuestClose(i) => self.vsock(i).peer_shutdown(SimShutdown::Both),
            Step::DestinationFin(i) => self.destination(i).peer_shutdown(SimShutdown::Send),
            Step::DestinationReset(i) => self.destination(i).peer_reset(),
            Step::OwnerAbort(i) => {
                self.fwd.abort(self.ids[&i]).expect("abort");
            }
            Step::AbortAll => {
                self.fwd.abort_all().expect("abort_all");
            }
        }
    }

    fn observe(&mut self) -> Observation {
        let fwd = &self.fwd;
        observe_port(fwd, &self.ids, &self.pairs, || {
            fwd.run_until_idle();
        })
    }
}

fn assert_equivalent(steps: &[Step]) {
    let mut kernel = KernelRig::new();
    let mut sim = SimRig::new();
    for (n, step) in steps.iter().enumerate() {
        kernel.apply(*step);
        sim.apply(*step);
        let (k, s) = (kernel.observe(), sim.observe());
        assert_eq!(k, s, "step {n} ({step:?}): the sim arbiter diverges from the real forwarder");
    }
    let end = kernel.observe();
    assert_eq!(
        (end.inventory.live_pairs, end.inventory.live_watches, end.inventory.queued_ends),
        (0, 0, 0)
    );
}

#[test]
fn every_end_class_drives_both_adapters_to_the_same_facts() {
    use Step::{
        AbortAll, Attach, DestinationFin, DestinationReset, GuestClose, GuestHalfClose, OwnerAbort,
        Watch,
    };
    assert_equivalent(&[
        Attach(0),
        Attach(1),
        Watch(2),
        Attach(3),
        // A clean end, one direction at a time: the pair completes.
        GuestHalfClose(0),
        DestinationFin(0),
        // A destination reset: SocketError on half B.
        DestinationReset(1),
        // A watched dial's guest half-close.
        GuestHalfClose(2),
        // The owner's abort.
        OwnerAbort(3),
        // A guest close with its direction open: PeerAbort.
        Attach(4),
        GuestClose(4),
        // A destination FIN then reset (EPIPE in CLOSE_WAIT).
        Attach(5),
        DestinationFin(5),
        DestinationReset(5),
        // An abort of an already-ended id changes nothing.
        OwnerAbort(0),
        // Everything still live, at once.
        Attach(6),
        Watch(7),
        Attach(8),
        AbortAll,
    ]);
}

#[test]
fn sixty_four_flows_and_their_teardown_count_the_same_on_both_adapters() {
    let mut steps = Vec::new();
    for i in 0..64 {
        steps.push(if i % 4 == 3 { Step::Watch(i) } else { Step::Attach(i) });
    }
    for i in (0..64).step_by(5) {
        steps.push(Step::GuestClose(i));
    }
    steps.push(Step::AbortAll);
    assert_equivalent(&steps);
}
