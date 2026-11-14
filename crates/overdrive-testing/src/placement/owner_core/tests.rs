//! Seeded and example tests of `OwnerCore` through its pinned surface:
//! `handle(Event) -> Vec<Effect>` and `observe()`.
//!
//! Contract: a shared guest serves attached clients one at a time in arrival
//! order, never cancels a running test, is discarded after a failed, lost,
//! bounded or detached-from exec, exits only when idle, and ignores every event
//! that is not enabled. The properties are asserted against an oracle built
//! from the events fed and the effects returned (never from the core's own
//! bookkeeping); the examples pin the effect lists of the named scenarios.
//!
//! The core has no pinned `Debug` or `PartialEq`, so the tests compare through
//! their own mirror types.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::doc_markdown,
    reason = "acceptance tests assert their own preconditions and name protocol terms"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use proptest::prelude::*;

use super::{AttachId, Effect, Event, GuestView, OwnerCore, Verdict};

// ----- mirror types: the core's enums carry no derives ----------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V {
    Exit(i32),
    BootFailed,
    GuestLost,
    ExecBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fx {
    StartBoot,
    StartExec(u32),
    Reply(u32, V),
    DiscardGuest,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum G {
    NoGuest,
    Booting,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snap {
    guest: G,
    generation: u32,
    in_flight: Option<u32>,
    queue: Vec<u32>,
    stopped: bool,
}

/// The model's client numbers are 1..=3 in the conformance driver; the tests
/// use a wider range, so they keep their own number for each `AttachId`.
fn number(id: AttachId) -> u32 {
    (0..=64_u32).find(|n| AttachId::new(*n) == id).expect("test ids are below 64")
}

fn id(n: u32) -> AttachId {
    AttachId::new(n)
}

fn snap(core: &OwnerCore) -> Snap {
    let observed = core.observe();
    Snap {
        guest: match observed.guest {
            GuestView::NoGuest => G::NoGuest,
            GuestView::Booting => G::Booting,
            GuestView::Ready => G::Ready,
        },
        generation: observed.generation,
        in_flight: observed.in_flight.map(number),
        queue: observed.queue.into_iter().map(number).collect(),
        stopped: observed.stopped,
    }
}

fn mirror(effects: Vec<Effect>) -> Vec<Fx> {
    effects
        .into_iter()
        .map(|effect| match effect {
            Effect::StartBoot => Fx::StartBoot,
            Effect::StartExec(c) => Fx::StartExec(number(c)),
            Effect::Reply(c, verdict) => Fx::Reply(
                number(c),
                match verdict {
                    Verdict::Exit(status) => V::Exit(status),
                    Verdict::BootFailed => V::BootFailed,
                    Verdict::GuestLost => V::GuestLost,
                    Verdict::ExecBound => V::ExecBound,
                },
            ),
            Effect::DiscardGuest => Fx::DiscardGuest,
            Effect::Exit => Fx::Exit,
        })
        .collect()
}

fn feed(core: &mut OwnerCore, event: Event) -> Vec<Fx> {
    mirror(core.handle(event))
}

/// An owner core with `clients` attached, the boot finished and the first exec
/// in flight (the shape most examples start from).
fn serving(clients: &[u32]) -> OwnerCore {
    let mut core = OwnerCore::new();
    for &client in clients {
        let _ = core.handle(Event::Attached(id(client)));
    }
    let _ = core.handle(Event::BootFinished(true));
    core
}

fn blank(generation: u32, guest: G, in_flight: Option<u32>, queue: &[u32]) -> Snap {
    Snap { guest, generation, in_flight, queue: queue.to_vec(), stopped: false }
}

// ----- a fresh owner ---------------------------------------------------------

/// CONTRACT_SHAPE: pure-function (a new core observes the empty state).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_new_owner_has_no_guest_no_clients_and_has_not_stopped() {
    let core = OwnerCore::new();
    assert_eq!(snap(&core), blank(0, G::NoGuest, None, &[]));
}

// ----- examples: the named scenarios ----------------------------------------

/// Three clients attach, the guest boots once, each is served in arrival order
/// and the guest stays for the next.
/// CONTRACT_SHAPE: bounded-change (the owner's observable state and effects).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn three_clients_share_one_guest_and_are_served_in_arrival_order() {
    let mut core = OwnerCore::new();
    assert_eq!(feed(&mut core, Event::Attached(id(1))), vec![Fx::StartBoot]);
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![]);
    assert_eq!(feed(&mut core, Event::Attached(id(3))), vec![]);
    assert_eq!(snap(&core), blank(1, G::Booting, None, &[1, 2, 3]));

    assert_eq!(feed(&mut core, Event::BootFinished(true)), vec![Fx::StartExec(1)]);
    assert_eq!(snap(&core), blank(1, G::Ready, Some(1), &[2, 3]));

    let after_first = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(after_first.contains(&Fx::Reply(1, V::Exit(0))), "{after_first:?}");
    assert!(after_first.contains(&Fx::StartExec(2)), "{after_first:?}");
    assert!(!after_first.contains(&Fx::DiscardGuest), "a passing test leaves the guest");
    let after_second = feed(&mut core, Event::ExecFinished(id(2), 0));
    assert!(
        after_second.contains(&Fx::Reply(2, V::Exit(0)))
            && after_second.contains(&Fx::StartExec(3))
    );
    let after_third = feed(&mut core, Event::ExecFinished(id(3), 0));
    assert_eq!(after_third, vec![Fx::Reply(3, V::Exit(0))]);
    assert_eq!(snap(&core), blank(1, G::Ready, None, &[]));
}

/// A failed test is answered with its status, its guest is discarded and the
/// waiting test is served on a replacement started in the same event.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_failed_test_discards_its_guest_and_the_next_test_gets_a_replacement() {
    let mut core = serving(&[1, 2, 3]);
    let effects = feed(&mut core, Event::ExecFinished(id(1), 1));
    assert!(effects.contains(&Fx::Reply(1, V::Exit(1))), "{effects:?}");
    assert!(effects.contains(&Fx::DiscardGuest), "{effects:?}");
    assert!(
        effects.contains(&Fx::StartBoot),
        "the replacement boots in the same event: {effects:?}"
    );
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2, 3]));
    assert_eq!(feed(&mut core, Event::BootFinished(true)), vec![Fx::StartExec(2)]);
}

/// A lost guest answers the running test and is replaced for the waiting one.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_guest_lost_during_a_test_answers_it_and_is_replaced_for_the_next() {
    let mut core = serving(&[1, 2]);
    let effects = feed(&mut core, Event::GuestLost);
    assert!(effects.contains(&Fx::Reply(1, V::GuestLost)), "{effects:?}");
    assert!(effects.contains(&Fx::StartBoot), "{effects:?}");
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// A guest lost while idle is observed as no guest and replaced at the next
/// attach, with generation + 1.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_guest_lost_while_idle_is_replaced_at_the_next_attach() {
    let mut core = serving(&[1]);
    let _ = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert_eq!(snap(&core), blank(1, G::Ready, None, &[]));
    let lost = feed(&mut core, Event::GuestLost);
    assert!(!lost.iter().any(|fx| matches!(fx, Fx::Reply(..))), "nobody is owed a reply: {lost:?}");
    assert_eq!(snap(&core), blank(1, G::NoGuest, None, &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![Fx::StartBoot]);
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// A client that goes away while its test runs does not free the slot; the
/// exec runs on, its result reaches nobody and the guest is discarded when it
/// ends, whatever its verdict.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_client_that_goes_away_while_its_test_runs_taints_the_guest_and_is_never_answered() {
    let mut core = serving(&[1]);
    assert_eq!(feed(&mut core, Event::ClientGone(id(1))), vec![], "the exec is never cancelled");
    assert_eq!(snap(&core), blank(1, G::Ready, Some(1), &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![], "the next test waits for the slot");
    assert_eq!(snap(&core), blank(1, G::Ready, Some(1), &[2]));
    let ended = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(
        !ended.iter().any(|fx| matches!(fx, Fx::Reply(1, _))),
        "a detached result reaches nobody: {ended:?}"
    );
    assert!(ended.contains(&Fx::DiscardGuest), "a passing detached exec still taints: {ended:?}");
    assert!(ended.contains(&Fx::StartBoot), "{ended:?}");
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// A queued client that goes away is removed and is never served or answered.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_queued_client_that_goes_away_is_removed_and_never_served() {
    let mut core = serving(&[1, 2, 3]);
    assert_eq!(feed(&mut core, Event::ClientGone(id(2))), vec![]);
    assert_eq!(snap(&core), blank(1, G::Ready, Some(1), &[3]));
    let effects = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(
        effects.contains(&Fx::StartExec(3))
            && !effects.iter().any(|fx| matches!(fx, Fx::StartExec(2) | Fx::Reply(2, _)))
    );
}

/// A boot failure answers only the head of the queue and starts a new boot for
/// the next client at once; the failed client is not retried.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_boot_failure_answers_only_the_head_and_starts_a_new_boot_for_the_next() {
    let mut core = OwnerCore::new();
    let _ = core.handle(Event::Attached(id(1)));
    let _ = core.handle(Event::Attached(id(2)));
    let effects = feed(&mut core, Event::BootFinished(false));
    assert!(effects.contains(&Fx::Reply(1, V::BootFailed)), "{effects:?}");
    assert!(effects.contains(&Fx::StartBoot), "{effects:?}");
    assert!(!effects.iter().any(|fx| matches!(fx, Fx::Reply(2, _))), "{effects:?}");
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// With nobody waiting a failed boot leaves no guest and notifies nobody; the
/// next attach boots generation + 1.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_boot_that_fails_with_nobody_waiting_leaves_no_guest_and_notifies_nobody() {
    let mut core = OwnerCore::new();
    let _ = core.handle(Event::Attached(id(1)));
    assert_eq!(feed(&mut core, Event::ClientGone(id(1))), vec![]);
    assert_eq!(feed(&mut core, Event::BootFinished(false)), vec![]);
    assert_eq!(snap(&core), blank(1, G::NoGuest, None, &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![Fx::StartBoot]);
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// A hung exec ends at its bound: the client is answered `ExecBound`, the
/// guest is discarded and the waiting client is served on a replacement.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn an_exec_that_reaches_its_bound_is_answered_exec_bound_and_its_guest_discarded() {
    let mut core = serving(&[1, 2]);
    let effects = feed(&mut core, Event::ExecBoundElapsed(id(1)));
    assert!(effects.contains(&Fx::Reply(1, V::ExecBound)), "{effects:?}");
    assert!(effects.contains(&Fx::DiscardGuest), "{effects:?}");
    assert!(effects.contains(&Fx::StartBoot), "{effects:?}");
    assert_eq!(snap(&core), blank(2, G::Booting, None, &[2]));
}

/// The late result of the exec that reached its bound reaches nobody.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn the_late_result_of_an_exec_that_reached_its_bound_changes_nothing() {
    let mut core = serving(&[1, 2]);
    let _ = core.handle(Event::ExecBoundElapsed(id(1)));
    let before = snap(&core);
    assert_eq!(feed(&mut core, Event::ExecFinished(id(1), 0)), vec![]);
    assert_eq!(snap(&core), before);
}

/// An exec whose client was killed returns after the next client attached: its
/// result reaches nobody and the new client is not answered by it.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_late_result_from_a_detached_exec_reaches_nobody_not_even_the_next_client() {
    let mut core = serving(&[1]);
    let _ = core.handle(Event::ClientGone(id(1)));
    let _ = core.handle(Event::Attached(id(2)));
    let ended = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(!ended.iter().any(|fx| matches!(fx, Fx::Reply(..))), "{ended:?}");
}

/// The owner exits only when nothing is queued, nothing runs and nobody is
/// attached; a detached exec still counts as running.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn an_owner_exits_when_idle_and_never_while_work_is_queued_or_running() {
    let mut core = OwnerCore::new();
    let _ = core.handle(Event::Attached(id(1)));
    let _ = core.handle(Event::Attached(id(2)));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![], "tests are queued");
    let _ = core.handle(Event::BootFinished(true));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![], "a test is running and one waits");
    let _ = core.handle(Event::ExecFinished(id(1), 0));
    let _ = core.handle(Event::ExecFinished(id(2), 0));
    assert_eq!(snap(&core), blank(1, G::Ready, None, &[]));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![Fx::Exit]);
    assert_eq!(snap(&core), Snap { stopped: true, ..blank(1, G::NoGuest, None, &[]) });
}

/// A detached exec is still in flight for the idle exit.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_detached_exec_still_counts_as_running_for_the_idle_exit() {
    let mut core = serving(&[1]);
    let _ = core.handle(Event::ClientGone(id(1)));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![]);
    assert!(!snap(&core).stopped);
}

/// After the owner exits every event is ignored.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn after_the_owner_exits_every_event_is_ignored() {
    let mut core = OwnerCore::new();
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![Fx::Exit]);
    let stopped = snap(&core);
    for event in [
        Event::Attached(id(1)),
        Event::ClientGone(id(1)),
        Event::BootFinished(true),
        Event::ExecFinished(id(1), 0),
        Event::GuestLost,
        Event::ExecBoundElapsed(id(1)),
        Event::IdleElapsed,
    ] {
        assert_eq!(feed(&mut core, event), vec![]);
        assert_eq!(snap(&core), stopped);
    }
}

// ----- seeded schedules ------------------------------------------------------

/// One step of a schedule. `Pick` selects among the ids seen so far, with one
/// extra slot for an id the core has never seen.
#[derive(Debug, Clone)]
enum Op {
    Attach,
    Gone(usize),
    Boot(bool),
    Done(usize, bool),
    Lost,
    Bound(usize),
    Idle,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Attach),
        2 => any::<usize>().prop_map(Op::Gone),
        3 => any::<bool>().prop_map(Op::Boot),
        3 => (any::<usize>(), any::<bool>()).prop_map(|(c, pass)| Op::Done(c, pass)),
        1 => Just(Op::Lost),
        1 => any::<usize>().prop_map(Op::Bound),
        1 => Just(Op::Idle),
    ]
}

const UNSEEN: u32 = 999;

/// The oracle: what the events fed and the effects returned imply, kept
/// without reading the core's own state.
#[derive(Default)]
struct Oracle {
    next_id: u32,
    seen: Vec<u32>,
    gone: BTreeSet<u32>,
    replied: BTreeMap<u32, u32>,
    arrival: VecDeque<u32>,
    running: Option<u32>,
    bad_generations: BTreeSet<u32>,
    stopped: bool,
}

impl Oracle {
    fn pick(&self, selector: usize) -> u32 {
        let slots = self.seen.len() + 1;
        self.seen.get(selector % slots).copied().unwrap_or(UNSEEN)
    }
}

fn event_of(oracle: &mut Oracle, op: &Op) -> Event {
    match op {
        Op::Attach => {
            oracle.next_id += 1;
            Event::Attached(id(oracle.next_id))
        }
        Op::Gone(c) => Event::ClientGone(id(oracle.pick(*c))),
        Op::Boot(ok) => Event::BootFinished(*ok),
        Op::Done(c, pass) => Event::ExecFinished(id(oracle.pick(*c)), i32::from(!*pass)),
        Op::Lost => Event::GuestLost,
        Op::Bound(c) => Event::ExecBoundElapsed(id(oracle.pick(*c))),
        Op::Idle => Event::IdleElapsed,
    }
}

/// Applies the event, checks every invariant of the model against the oracle
/// and updates the oracle. Panics with the invariant's name.
fn step(core: &mut OwnerCore, oracle: &mut Oracle, event: Event) {
    let before = snap(core);
    if oracle.stopped {
        assert!(mirror(core.handle(event)).is_empty(), "an event after Exit returned effects");
        assert_eq!(snap(core), before, "an event after Exit changed the observation");
        return;
    }
    // What this event means to the oracle, decided from the oracle alone.
    let mut taints = false;
    let mut ends_running = false;
    match &event {
        Event::Attached(c) => {
            let c = number(*c);
            oracle.seen.push(c);
            oracle.arrival.push_back(c);
        }
        Event::ClientGone(c) => {
            let c = number(*c);
            let queued = oracle.arrival.contains(&c);
            if queued || oracle.running == Some(c) {
                if queued {
                    oracle.arrival.retain(|x| *x != c);
                } else {
                    taints = true;
                }
                oracle.gone.insert(c);
            }
        }
        Event::ExecFinished(c, status) => {
            if oracle.running == Some(number(*c)) {
                taints = *status != 0;
                ends_running = true;
            }
        }
        Event::ExecBoundElapsed(c) => {
            if oracle.running == Some(number(*c)) {
                taints = true;
                ends_running = true;
            }
        }
        Event::GuestLost => {
            if before.guest == G::Ready {
                taints = true;
                ends_running = oracle.running.is_some();
            }
        }
        Event::BootFinished(_) | Event::IdleElapsed => {}
    }
    if taints {
        oracle.bad_generations.insert(before.generation);
    }
    let was_running = oracle.running;
    if ends_running {
        oracle.running = None;
    }
    let idle_now = before.queue.is_empty() && before.in_flight.is_none();
    let is_idle_event = matches!(event, Event::IdleElapsed);

    let effects = mirror(core.handle(event));
    let after = snap(core);

    // generation: monotone, one more per StartBoot
    let boots = u32::try_from(effects.iter().filter(|fx| matches!(fx, Fx::StartBoot)).count())
        .expect("a handful of effects");
    assert_eq!(
        after.generation,
        before.generation + boots,
        "generation grows exactly by StartBoot: {effects:?}"
    );

    for fx in &effects {
        match *fx {
            Fx::StartExec(c) => {
                // I_SERIAL: never a second exec while one is in flight
                assert!(
                    oracle.running.is_none(),
                    "I_SERIAL: StartExec({c}) while {was_running:?} runs"
                );
                // arrival order, and never a client that went away
                let head = oracle.arrival.pop_front();
                assert_eq!(head, Some(c), "serving order is arrival order");
                oracle.running = Some(c);
            }
            Fx::Reply(c, _) => {
                // I_DELIVER_ONLY_ATTACHED: not to a client that went away, once each
                assert!(!oracle.gone.contains(&c), "a reply reached client {c} who went away");
                assert!(oracle.seen.contains(&c), "a reply reached a client never attached");
                *oracle.replied.entry(c).or_insert(0) += 1;
                assert_eq!(oracle.replied[&c], 1, "client {c} was answered twice");
            }
            Fx::Exit => {
                assert!(is_idle_event && idle_now, "Exit while work remained: before={before:?}");
                oracle.stopped = true;
            }
            Fx::StartBoot | Fx::DiscardGuest => {}
        }
    }

    // structural invariants of the observation
    assert_eq!(
        after.in_flight, oracle.running,
        "in_flight is the exec the effects started and no event ended"
    );
    assert!(
        after.in_flight.is_none_or(|c| !after.queue.contains(&c)),
        "the running client is not also queued"
    );
    assert_eq!(
        after.queue.iter().copied().collect::<BTreeSet<_>>().len(),
        after.queue.len(),
        "the queue holds a client once"
    );
    let expected_queue: Vec<u32> = oracle.arrival.iter().copied().collect();
    // queued = arrived, not removed, not yet started
    assert_eq!(
        after.queue, expected_queue,
        "the queue is the arrival order minus removed and started clients"
    );
    // I_NO_REUSE: a ready, idle guest is never a bad generation
    if after.guest == G::Ready && after.in_flight.is_none() {
        assert!(
            !oracle.bad_generations.contains(&after.generation),
            "I_NO_REUSE: generation {} is bad",
            after.generation
        );
    }
    if after.stopped {
        assert!(after.guest == G::NoGuest && after.in_flight.is_none() && after.queue.is_empty());
    }
}

fn run_schedule(ops: &[Op]) -> (OwnerCore, Oracle) {
    let mut core = OwnerCore::new();
    let mut oracle = Oracle::default();
    for op in ops {
        let event = event_of(&mut oracle, op);
        step(&mut core, &mut oracle, event);
    }
    (core, oracle)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For every schedule of attach, gone, boot, exec, loss, bound and idle
    /// events the core serves one client at a time in arrival order, never
    /// answers a client that went away, never reuses a bad generation and
    /// never exits with work remaining.
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[test]
    #[ignore = "pending DELIVER: module-tests-share-one-guest"]
    fn seeded_event_schedules_keep_the_owner_invariants(ops in proptest::collection::vec(op(), 0..60)) {
        let _ = run_schedule(&ops);
    }

    /// When the environment answers every boot and exec, every client that did
    /// not go away is answered exactly once (the model's `EveryClientTerminates`).
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[test]
    #[ignore = "pending DELIVER: module-tests-share-one-guest"]
    fn every_attached_client_that_stays_is_answered_once_when_the_environment_answers_everything(
        ops in proptest::collection::vec(op(), 0..40),
    ) {
        let (mut core, mut oracle) = run_schedule(&ops);
        for _ in 0..200 {
            let now = snap(&core);
            let event = match (now.stopped, now.guest, now.in_flight) {
                (true, ..) => break,
                (_, G::Booting, _) => Event::BootFinished(true),
                (_, _, Some(c)) => Event::ExecFinished(id(c), 0),
                _ => break,
            };
            step(&mut core, &mut oracle, event);
        }
        for client in oracle.seen.clone() {
            if oracle.gone.contains(&client) {
                continue;
            }
            prop_assert_eq!(
                oracle.replied.get(&client).copied(),
                Some(1),
                "client {} was not answered exactly once",
                client
            );
        }
    }

    /// An event that is not enabled in the state the schedule reached returns
    /// no effects and changes nothing, in every such state and for every such
    /// event.
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[test]
    #[ignore = "pending DELIVER: module-tests-share-one-guest"]
    fn events_that_are_not_enabled_change_nothing(ops in proptest::collection::vec(op(), 0..60)) {
        let (mut core, oracle) = run_schedule(&ops);
        let now = snap(&core);
        let mut stale: Vec<Event> = Vec::new();
        if now.stopped {
            stale.extend([
                Event::Attached(id(500)),
                Event::ClientGone(id(1)),
                Event::BootFinished(true),
                Event::ExecFinished(id(1), 0),
                Event::GuestLost,
                Event::ExecBoundElapsed(id(1)),
                Event::IdleElapsed,
            ]);
        } else {
            let known: BTreeSet<u32> = now.queue.iter().copied().chain(now.in_flight).collect();
            // a client that is neither queued nor running: never seen, or already answered
            let outside: Vec<u32> = std::iter::once(UNSEEN)
                .chain(oracle.replied.keys().copied())
                .filter(|c| !known.contains(c))
                .collect();
            for c in &outside {
                stale.push(Event::ClientGone(id(*c)));
                stale.push(Event::ExecFinished(id(*c), 0));
                stale.push(Event::ExecBoundElapsed(id(*c)));
            }
            for c in &known {
                stale.push(Event::Attached(id(*c)));
                if now.in_flight != Some(*c) {
                    stale.push(Event::ExecFinished(id(*c), 0));
                    stale.push(Event::ExecBoundElapsed(id(*c)));
                }
            }
            if now.guest != G::Ready {
                stale.push(Event::GuestLost);
            }
            if now.guest != G::Booting {
                stale.push(Event::BootFinished(true));
                stale.push(Event::BootFinished(false));
            }
            if !now.queue.is_empty() || now.in_flight.is_some() {
                stale.push(Event::IdleElapsed);
            }
        }
        for event in stale {
            let effects = mirror(core.handle(event));
            prop_assert!(effects.is_empty(), "a stale event returned {:?} in {:?}", effects, now);
            prop_assert_eq!(snap(&core), now.clone());
        }
    }
}
