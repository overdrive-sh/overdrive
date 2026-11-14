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
//! The core's types derive `Clone, Debug, PartialEq, Eq` (the pinned shape), so
//! the tests compare the core's own values.

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

use super::{AttachId, Effect, Event, GuestView, OwnerCore, OwnerObservation, Verdict};

fn id(n: u32) -> AttachId {
    AttachId::new(n)
}

fn feed(core: &mut OwnerCore, event: Event) -> Vec<Effect> {
    core.handle(event)
}

/// True when `effects` answer `client` with any verdict.
fn answers(effects: &[Effect], client: u32) -> bool {
    effects.iter().any(|fx| matches!(fx, Effect::Reply(c, _) if *c == id(client)))
}

/// True when `effects` start `client`'s exec.
fn starts(effects: &[Effect], client: u32) -> bool {
    effects.iter().any(|fx| matches!(fx, Effect::StartExec(c) if *c == id(client)))
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

fn blank(
    generation: u32,
    guest: GuestView,
    in_flight: Option<u32>,
    queue: &[u32],
) -> OwnerObservation {
    OwnerObservation {
        guest,
        generation,
        in_flight: in_flight.map(id),
        queue: queue.iter().copied().map(id).collect(),
        stopped: false,
    }
}

// ----- a fresh owner ---------------------------------------------------------

/// CONTRACT_SHAPE: pure-function (a new core observes the empty state).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_new_owner_has_no_guest_no_clients_and_has_not_stopped() {
    let core = OwnerCore::new();
    assert_eq!(core.observe(), blank(0, GuestView::NoGuest, None, &[]));
}

// ----- examples: the named scenarios ----------------------------------------

/// Three clients attach, the guest boots once, each is served in arrival order
/// and the guest stays for the next.
/// CONTRACT_SHAPE: bounded-change (the owner's observable state and effects).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn three_clients_share_one_guest_and_are_served_in_arrival_order() {
    let mut core = OwnerCore::new();
    assert_eq!(feed(&mut core, Event::Attached(id(1))), vec![Effect::StartBoot]);
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![]);
    assert_eq!(feed(&mut core, Event::Attached(id(3))), vec![]);
    assert_eq!(core.observe(), blank(1, GuestView::Booting, None, &[1, 2, 3]));

    assert_eq!(feed(&mut core, Event::BootFinished(true)), vec![Effect::StartExec(id(1))]);
    assert_eq!(core.observe(), blank(1, GuestView::Ready, Some(1), &[2, 3]));

    let after_first = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(after_first.contains(&Effect::Reply(id(1), Verdict::Exit(0))), "{after_first:?}");
    assert!(after_first.contains(&Effect::StartExec(id(2))), "{after_first:?}");
    assert!(!after_first.contains(&Effect::DiscardGuest), "a passing test leaves the guest");
    let after_second = feed(&mut core, Event::ExecFinished(id(2), 0));
    assert!(
        after_second.contains(&Effect::Reply(id(2), Verdict::Exit(0)))
            && after_second.contains(&Effect::StartExec(id(3)))
    );
    let after_third = feed(&mut core, Event::ExecFinished(id(3), 0));
    assert_eq!(after_third, vec![Effect::Reply(id(3), Verdict::Exit(0))]);
    assert_eq!(core.observe(), blank(1, GuestView::Ready, None, &[]));
}

/// A failed test is answered with its status, its guest is discarded and the
/// waiting test is served on a replacement started in the same event.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_failed_test_discards_its_guest_and_the_next_test_gets_a_replacement() {
    let mut core = serving(&[1, 2, 3]);
    let effects = feed(&mut core, Event::ExecFinished(id(1), 1));
    assert!(effects.contains(&Effect::Reply(id(1), Verdict::Exit(1))), "{effects:?}");
    assert!(effects.contains(&Effect::DiscardGuest), "{effects:?}");
    assert!(
        effects.contains(&Effect::StartBoot),
        "the replacement boots in the same event: {effects:?}"
    );
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2, 3]));
    assert_eq!(feed(&mut core, Event::BootFinished(true)), vec![Effect::StartExec(id(2))]);
}

/// A lost guest answers the running test and is replaced for the waiting one.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_guest_lost_during_a_test_answers_it_and_is_replaced_for_the_next() {
    let mut core = serving(&[1, 2]);
    let effects = feed(&mut core, Event::GuestLost);
    assert!(effects.contains(&Effect::Reply(id(1), Verdict::GuestLost)), "{effects:?}");
    assert!(effects.contains(&Effect::StartBoot), "{effects:?}");
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
}

/// A guest lost while idle is observed as no guest and replaced at the next
/// attach, with generation + 1.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_guest_lost_while_idle_is_replaced_at_the_next_attach() {
    let mut core = serving(&[1]);
    let _ = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert_eq!(core.observe(), blank(1, GuestView::Ready, None, &[]));
    let lost = feed(&mut core, Event::GuestLost);
    assert!(
        !lost.iter().any(|fx| matches!(fx, Effect::Reply(..))),
        "nobody is owed a reply: {lost:?}"
    );
    assert_eq!(core.observe(), blank(1, GuestView::NoGuest, None, &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![Effect::StartBoot]);
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
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
    assert_eq!(core.observe(), blank(1, GuestView::Ready, Some(1), &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![], "the next test waits for the slot");
    assert_eq!(core.observe(), blank(1, GuestView::Ready, Some(1), &[2]));
    let ended = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(!answers(&ended, 1), "a detached result reaches nobody: {ended:?}");
    assert!(
        ended.contains(&Effect::DiscardGuest),
        "a passing detached exec still taints: {ended:?}"
    );
    assert!(ended.contains(&Effect::StartBoot), "{ended:?}");
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
}

/// A queued client that goes away is removed and is never served or answered.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_queued_client_that_goes_away_is_removed_and_never_served() {
    let mut core = serving(&[1, 2, 3]);
    assert_eq!(feed(&mut core, Event::ClientGone(id(2))), vec![]);
    assert_eq!(core.observe(), blank(1, GuestView::Ready, Some(1), &[3]));
    let effects = feed(&mut core, Event::ExecFinished(id(1), 0));
    assert!(
        effects.contains(&Effect::StartExec(id(3)))
            && !starts(&effects, 2)
            && !answers(&effects, 2)
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
    assert!(effects.contains(&Effect::Reply(id(1), Verdict::BootFailed)), "{effects:?}");
    assert!(effects.contains(&Effect::StartBoot), "{effects:?}");
    assert!(!answers(&effects, 2), "{effects:?}");
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
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
    assert_eq!(core.observe(), blank(1, GuestView::NoGuest, None, &[]));
    assert_eq!(feed(&mut core, Event::Attached(id(2))), vec![Effect::StartBoot]);
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
}

/// A hung exec ends at its bound: the client is answered `ExecBound`, the
/// guest is discarded and the waiting client is served on a replacement.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn an_exec_that_reaches_its_bound_is_answered_exec_bound_and_its_guest_discarded() {
    let mut core = serving(&[1, 2]);
    let effects = feed(&mut core, Event::ExecBoundElapsed(id(1)));
    assert!(effects.contains(&Effect::Reply(id(1), Verdict::ExecBound)), "{effects:?}");
    assert!(effects.contains(&Effect::DiscardGuest), "{effects:?}");
    assert!(effects.contains(&Effect::StartBoot), "{effects:?}");
    assert_eq!(core.observe(), blank(2, GuestView::Booting, None, &[2]));
}

/// The late result of the exec that reached its bound reaches nobody.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn the_late_result_of_an_exec_that_reached_its_bound_changes_nothing() {
    let mut core = serving(&[1, 2]);
    let _ = core.handle(Event::ExecBoundElapsed(id(1)));
    let before = core.observe();
    assert_eq!(feed(&mut core, Event::ExecFinished(id(1), 0)), vec![]);
    assert_eq!(core.observe(), before);
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
    assert!(!ended.iter().any(|fx| matches!(fx, Effect::Reply(..))), "{ended:?}");
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
    assert_eq!(core.observe(), blank(1, GuestView::Ready, None, &[]));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![Effect::Exit]);
    assert_eq!(
        core.observe(),
        OwnerObservation { stopped: true, ..blank(1, GuestView::NoGuest, None, &[]) }
    );
}

/// A detached exec is still in flight for the idle exit.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_detached_exec_still_counts_as_running_for_the_idle_exit() {
    let mut core = serving(&[1]);
    let _ = core.handle(Event::ClientGone(id(1)));
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![]);
    assert!(!core.observe().stopped);
}

/// After the owner exits every event is ignored.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn after_the_owner_exits_every_event_is_ignored() {
    let mut core = OwnerCore::new();
    assert_eq!(feed(&mut core, Event::IdleElapsed), vec![Effect::Exit]);
    let stopped = core.observe();
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
        assert_eq!(core.observe(), stopped);
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

fn unseen() -> AttachId {
    id(999)
}

/// The oracle: what the events fed and the effects returned imply, kept
/// without reading the core's own state.
#[derive(Default)]
struct Oracle {
    next_id: u32,
    seen: Vec<AttachId>,
    gone: BTreeSet<AttachId>,
    replied: BTreeMap<AttachId, u32>,
    arrival: VecDeque<AttachId>,
    running: Option<AttachId>,
    bad_generations: BTreeSet<u32>,
    stopped: bool,
}

impl Oracle {
    fn pick(&self, selector: usize) -> AttachId {
        let slots = self.seen.len() + 1;
        self.seen.get(selector % slots).copied().unwrap_or_else(unseen)
    }
}

fn event_of(oracle: &mut Oracle, op: &Op) -> Event {
    match op {
        Op::Attach => {
            oracle.next_id += 1;
            Event::Attached(id(oracle.next_id))
        }
        Op::Gone(c) => Event::ClientGone(oracle.pick(*c)),
        Op::Boot(ok) => Event::BootFinished(*ok),
        Op::Done(c, pass) => Event::ExecFinished(oracle.pick(*c), i32::from(!*pass)),
        Op::Lost => Event::GuestLost,
        Op::Bound(c) => Event::ExecBoundElapsed(oracle.pick(*c)),
        Op::Idle => Event::IdleElapsed,
    }
}

/// Applies the event, checks every invariant of the model against the oracle
/// and updates the oracle. Panics with the invariant's name.
fn step(core: &mut OwnerCore, oracle: &mut Oracle, event: Event) {
    let before = core.observe();
    if oracle.stopped {
        assert!(core.handle(event).is_empty(), "an event after Exit returned effects");
        assert_eq!(core.observe(), before, "an event after Exit changed the observation");
        return;
    }
    // What this event means to the oracle, decided from the oracle alone.
    let mut taints = false;
    let mut ends_running = false;
    match &event {
        Event::Attached(c) => {
            oracle.seen.push(*c);
            oracle.arrival.push_back(*c);
        }
        Event::ClientGone(c) => {
            let queued = oracle.arrival.contains(c);
            if queued || oracle.running == Some(*c) {
                if queued {
                    oracle.arrival.retain(|x| x != c);
                } else {
                    taints = true;
                }
                oracle.gone.insert(*c);
            }
        }
        Event::ExecFinished(c, status) => {
            if oracle.running == Some(*c) {
                taints = *status != 0;
                ends_running = true;
            }
        }
        Event::ExecBoundElapsed(c) => {
            if oracle.running == Some(*c) {
                taints = true;
                ends_running = true;
            }
        }
        Event::GuestLost => {
            if before.guest == GuestView::Ready {
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

    let effects = core.handle(event);
    let after = core.observe();

    // generation: monotone, one more per StartBoot
    let boots = u32::try_from(effects.iter().filter(|fx| matches!(fx, Effect::StartBoot)).count())
        .expect("a handful of effects");
    assert_eq!(
        after.generation,
        before.generation + boots,
        "generation grows exactly by StartBoot: {effects:?}"
    );

    for fx in &effects {
        match fx {
            Effect::StartExec(c) => {
                // I_SERIAL: never a second exec while one is in flight
                assert!(
                    oracle.running.is_none(),
                    "I_SERIAL: StartExec({c:?}) while {was_running:?} runs"
                );
                // arrival order, and never a client that went away
                let head = oracle.arrival.pop_front();
                assert_eq!(head, Some(*c), "serving order is arrival order");
                oracle.running = Some(*c);
            }
            Effect::Reply(c, _) => {
                // I_DELIVER_ONLY_ATTACHED: not to a client that went away, once each
                assert!(!oracle.gone.contains(c), "a reply reached client {c:?} who went away");
                assert!(oracle.seen.contains(c), "a reply reached a client never attached");
                *oracle.replied.entry(*c).or_insert(0) += 1;
                assert_eq!(oracle.replied[c], 1, "client {c:?} was answered twice");
            }
            Effect::Exit => {
                assert!(is_idle_event && idle_now, "Exit while work remained: before={before:?}");
                oracle.stopped = true;
            }
            Effect::StartBoot | Effect::DiscardGuest => {}
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
    // queued = arrived, not removed, not yet started
    let expected_queue: Vec<AttachId> = oracle.arrival.iter().copied().collect();
    assert_eq!(
        after.queue, expected_queue,
        "the queue is the arrival order minus removed and started clients"
    );
    // I_NO_REUSE: a ready, idle guest is never a bad generation
    if after.guest == GuestView::Ready && after.in_flight.is_none() {
        assert!(
            !oracle.bad_generations.contains(&after.generation),
            "I_NO_REUSE: generation {} is bad",
            after.generation
        );
    }
    if after.stopped {
        assert!(
            after.guest == GuestView::NoGuest
                && after.in_flight.is_none()
                && after.queue.is_empty()
        );
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
            let now = core.observe();
            let event = match (now.stopped, &now.guest, now.in_flight) {
                (true, ..) => break,
                (_, GuestView::Booting, _) => Event::BootFinished(true),
                (_, _, Some(c)) => Event::ExecFinished(c, 0),
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
                "client {:?} was not answered exactly once",
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
        let now = core.observe();
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
            let known: BTreeSet<AttachId> =
                now.queue.iter().copied().chain(now.in_flight).collect();
            // a client that is neither queued nor running: never seen, or already answered
            let outside: Vec<AttachId> = std::iter::once(unseen())
                .chain(oracle.replied.keys().copied())
                .filter(|c| !known.contains(c))
                .collect();
            for c in &outside {
                stale.push(Event::ClientGone(*c));
                stale.push(Event::ExecFinished(*c, 0));
                stale.push(Event::ExecBoundElapsed(*c));
            }
            for c in &known {
                stale.push(Event::Attached(*c));
                if now.in_flight != Some(*c) {
                    stale.push(Event::ExecFinished(*c, 0));
                    stale.push(Event::ExecBoundElapsed(*c));
                }
            }
            if now.guest != GuestView::Ready {
                stale.push(Event::GuestLost);
            }
            if now.guest != GuestView::Booting {
                stale.push(Event::BootFinished(true));
                stale.push(Event::BootFinished(false));
            }
            if !now.queue.is_empty() || now.in_flight.is_some() {
                stale.push(Event::IdleElapsed);
            }
        }
        for event in stale {
            let effects = core.handle(event);
            prop_assert!(effects.is_empty(), "a stale event returned {:?} in {:?}", effects, now);
            prop_assert_eq!(core.observe(), now.clone());
        }
    }
}
