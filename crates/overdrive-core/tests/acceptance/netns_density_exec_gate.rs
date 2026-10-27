//! Acceptance properties for GH #295's paired EXEC admission capabilities.

#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_network::{
    GuestNetworkExecWiring, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_sim::adapters::clock::SimClock;
use proptest::prelude::*;

const COMPONENTS: [SharedGuestNetworkComponent; 9] = [
    SharedGuestNetworkComponent::Forwarder,
    SharedGuestNetworkComponent::FlowListeners,
    SharedGuestNetworkComponent::ControlListener,
    SharedGuestNetworkComponent::LegF,
    SharedGuestNetworkComponent::LegC,
    SharedGuestNetworkComponent::IpRules,
    SharedGuestNetworkComponent::IpSets,
    SharedGuestNetworkComponent::Dns,
    SharedGuestNetworkComponent::Supervisor,
];

/// Every post-attachment-cut fail-stop cause in declaration order (FD
/// § "Typed error taxonomy and who branches"). The gate is cause-agnostic,
/// so every member of the exact closed set produces the same terminal gate
/// transition and participates in the first-request-wins contract.
const CAUSES: [SharedGuestNetworkFailStopCause; 8] = [
    SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
    SharedGuestNetworkFailStopCause::LeakedQuiescenceHold,
    SharedGuestNetworkFailStopCause::ForeignQuiescenceHold,
    SharedGuestNetworkFailStopCause::SupervisorReturned,
    SharedGuestNetworkFailStopCause::SupervisorFailed,
    SharedGuestNetworkFailStopCause::SupervisorPanicked,
    SharedGuestNetworkFailStopCause::SupervisorCancelled,
    SharedGuestNetworkFailStopCause::RequestChannelClosed,
];
// The generated `FailStop(index)` range below indexes `CAUSES`.
const _: () = assert!(CAUSES.len() == 8);

#[derive(Clone, Copy, Debug)]
enum Operation {
    OpenAfterBoot,
    BeginRecovery(u8),
    CompleteAttempt(u16),
    FailStop(u8),
    Claim,
    DropClaim,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModelState {
    BootClosed,
    Open,
    Recovering {
        attempts: BTreeMap<SharedGuestNetworkComponent, u32>,
        joined_since_attempt: BTreeSet<SharedGuestNetworkComponent>,
        latest: SharedGuestNetworkComponent,
    },
    FailStop,
}

fn component_set(mask: u16) -> BTreeSet<SharedGuestNetworkComponent> {
    COMPONENTS
        .iter()
        .enumerate()
        .filter_map(|(index, component)| ((mask & (1 << index)) != 0).then_some(*component))
        .collect()
}

fn operation_strategy() -> impl Strategy<Value = Operation> {
    prop_oneof![
        Just(Operation::OpenAfterBoot),
        (0_u8..9).prop_map(Operation::BeginRecovery),
        (0_u16..(1_u16 << COMPONENTS.len())).prop_map(Operation::CompleteAttempt),
        (0_u8..8).prop_map(Operation::FailStop),
        Just(Operation::Claim),
        Just(Operation::DropClaim),
    ]
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: pure-function.
#[test]
fn shared_guest_network_admission_starts_closed_until_boot_read_back_completes() {
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));

    assert!(wiring.supervisor().is_boot_closed());
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn recovery_closes_new_exec_claims_and_full_read_back_reopens_them() {
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let gate = wiring.gate();
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot());

    let pre_detection = gate.claim_release().await.expect("Open admits an EXEC claim");
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::Forwarder));

    let waiting_gate = Arc::clone(&gate);
    let waiter = tokio::spawn(async move { waiting_gate.claim_release().await });
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "Recovering must not release a new command");

    assert!(supervisor.complete_attempt(BTreeSet::new()));
    let post_audit = waiter.await.expect("waiter task joins").expect("full audit reopens EXEC");
    drop(post_audit);
    drop(pre_detection);
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn fail_stop_refuses_waiting_and_future_exec_without_revoking_a_prior_claim() {
    let clock = SimClock::new();
    let wiring = GuestNetworkExecWiring::new(Arc::new(clock.clone()));
    let gate = wiring.gate();
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot());
    let prior_claim = gate.claim_release().await.expect("pre-detection claim is retained");
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::ControlListener));
    let before_fail_stop =
        supervisor.recovery_progress().pop().expect("Recovering exposes one immutable snapshot");

    let waiting_gate = Arc::clone(&gate);
    let recovering_waiter = tokio::spawn(async move { waiting_gate.claim_release().await });
    tokio::task::yield_now().await;
    assert!(!recovering_waiter.is_finished(), "Recovering keeps an existing waiter parked");

    clock.tick(Duration::from_secs(5));
    let receipt = supervisor
        .fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded)
        .expect("first fail-stop returns the one receipt");
    assert_eq!(receipt.component, SharedGuestNetworkComponent::ControlListener);
    assert_eq!(receipt.cause, SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
    assert_eq!(receipt.attempts, before_fail_stop.attempts);
    assert_eq!(receipt.elapsed, Duration::from_secs(5));
    assert!(supervisor.recovery_progress().is_empty(), "FailStop consumes only recovery state");
    assert!(
        supervisor.fail_stop(SharedGuestNetworkFailStopCause::SupervisorFailed).is_none(),
        "request emission is first-wins and idempotent"
    );
    assert!(
        recovering_waiter.await.expect("FailStop wakes the existing Recovering waiter").is_none(),
        "the woken waiter is refused rather than released"
    );
    assert!(gate.claim_release().await.is_none());
    drop(prior_claim);
}

proptest! {
    /// Outcome anchor: DISCUSS Elevator Pitch
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn generated_operation_sequences_match_the_gate_model(
        operations in prop::collection::vec(operation_strategy(), 1..64),
    ) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build generated gate runtime")
            .block_on(async move {
                let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
                let gate = wiring.gate();
                let supervisor = wiring.supervisor();
                let mut model = ModelState::BootClosed;
                let mut active_claims = Vec::new();
                let mut waiting_claims = Vec::new();

                for operation in operations {
                    match operation {
                        Operation::OpenAfterBoot => {
                            let accepted = supervisor.open_after_boot();
                            let expected = model == ModelState::BootClosed;
                            prop_assert_eq!(accepted, expected);
                            if expected {
                                model = ModelState::Open;
                            }
                        }
                        Operation::BeginRecovery(index) => {
                            let component = COMPONENTS[usize::from(index)];
                            let accepted = supervisor.begin_recovery(component);
                            let expected = match &model {
                                ModelState::Open => true,
                                ModelState::Recovering { attempts, .. } => {
                                    !attempts.contains_key(&component)
                                }
                                ModelState::BootClosed | ModelState::FailStop => false,
                            };
                            prop_assert_eq!(accepted, expected);
                            if expected {
                                if matches!(&model, ModelState::Open) {
                                    model = ModelState::Recovering {
                                        attempts: BTreeMap::from([(component, 0)]),
                                        joined_since_attempt: BTreeSet::new(),
                                        latest: component,
                                    };
                                } else if let ModelState::Recovering {
                                        attempts,
                                        joined_since_attempt,
                                        latest,
                                    } = &mut model
                                {
                                    attempts.insert(component, 0);
                                    joined_since_attempt.insert(component);
                                    *latest = component;
                                } else {
                                    unreachable!();
                                }
                            }
                        }
                        Operation::CompleteAttempt(mask) => {
                            let remaining = component_set(mask);
                            let accepted = supervisor.complete_attempt(remaining.clone());
                            let expected = matches!(&model, ModelState::Recovering { .. });
                            prop_assert_eq!(accepted, expected);
                            if expected {
                                let mut reopen = false;
                                if let ModelState::Recovering {
                                    attempts,
                                    joined_since_attempt,
                                    latest,
                                } = &mut model
                                {
                                    attempts.retain(|component, count| {
                                        if !remaining.contains(component) {
                                            return false;
                                        }
                                        if !joined_since_attempt.contains(component) {
                                            *count = count.saturating_add(1);
                                        }
                                        true
                                    });
                                    joined_since_attempt.clear();
                                    if let Some(first) = attempts.keys().next().copied() {
                                        *latest = first;
                                    } else {
                                        reopen = true;
                                    }
                                }
                                if reopen {
                                    model = ModelState::Open;
                                }
                            }
                        }
                        Operation::FailStop(index) => {
                            let cause = CAUSES[usize::from(index)];
                            let receipt = supervisor.fail_stop(cause);
                            if model == ModelState::FailStop {
                                prop_assert!(receipt.is_none());
                            } else {
                                let receipt = receipt
                                    .expect("the first terminal transition returns a receipt");
                                let (component, attempts) = match &model {
                                    ModelState::Recovering { attempts, latest, .. } => {
                                        (*latest, attempts[latest])
                                    }
                                    ModelState::BootClosed | ModelState::Open => {
                                        (SharedGuestNetworkComponent::Supervisor, 0)
                                    }
                                    ModelState::FailStop => unreachable!(),
                                };
                                prop_assert_eq!(receipt.component, component);
                                prop_assert_eq!(receipt.attempts, attempts);
                                prop_assert_eq!(receipt.cause, cause);
                                model = ModelState::FailStop;
                            }
                        }
                        Operation::Claim => match &model {
                            ModelState::Open => {
                                active_claims.push(
                                    gate.claim_release().await.expect("Open grants a claim"),
                                );
                            }
                            ModelState::FailStop => {
                                prop_assert!(gate.claim_release().await.is_none());
                            }
                            ModelState::BootClosed | ModelState::Recovering { .. } => {
                                let waiting_gate = Arc::clone(&gate);
                                let waiter = tokio::spawn(async move {
                                    waiting_gate.claim_release().await
                                });
                                tokio::task::yield_now().await;
                                prop_assert!(!waiter.is_finished());
                                waiting_claims.push(waiter);
                            }
                        },
                        Operation::DropClaim => {
                            active_claims.pop();
                        }
                    }

                    tokio::task::yield_now().await;
                    let mut waiting = 0;
                    while waiting < waiting_claims.len() {
                        if waiting_claims[waiting].is_finished() {
                            let waiter = waiting_claims.swap_remove(waiting);
                            let claim = waiter.await.expect("generated waiter joins");
                            match &model {
                                ModelState::Open => active_claims.push(
                                    claim.expect("a waiter woken into Open receives a claim"),
                                ),
                                ModelState::FailStop => {
                                    prop_assert!(claim.is_none());
                                }
                                ModelState::BootClosed | ModelState::Recovering { .. } => {
                                    prop_assert!(false, "closed state cannot finish a claim waiter");
                                }
                            }
                        } else {
                            waiting += 1;
                        }
                    }

                    prop_assert_eq!(supervisor.is_boot_closed(), model == ModelState::BootClosed);
                    let progress = supervisor.recovery_progress();
                    match &model {
                        ModelState::Recovering { attempts, .. } => {
                            prop_assert_eq!(progress.len(), attempts.len());
                            for ((component, count), observed) in attempts.iter().zip(&progress) {
                                prop_assert_eq!(observed.component, *component);
                                prop_assert_eq!(observed.attempts, *count);
                            }
                        }
                        ModelState::BootClosed | ModelState::Open | ModelState::FailStop => {
                            prop_assert!(progress.is_empty());
                        }
                    }
                }
                for waiter in waiting_claims {
                    waiter.abort();
                }
                drop(active_claims);
                Ok(())
            })?;
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-27 — One guest-command gate; every post-attachment-cut fail-stop
/// cause closes it and the first request wins.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn every_fail_stop_cause_is_closed_and_first_request_wins() {
    for component in COMPONENTS {
        for cause in CAUSES {
            let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
            let gate = wiring.gate();
            let supervisor = wiring.supervisor();
            assert!(supervisor.open_after_boot());
            assert!(supervisor.begin_recovery(component));
            let first = supervisor.fail_stop(cause).expect("first caller owns the request");

            assert_eq!(first.component, component);
            assert_eq!(first.cause, cause);
            assert!(
                supervisor.fail_stop(SharedGuestNetworkFailStopCause::SupervisorFailed).is_none()
            );
            assert!(supervisor.recovery_progress().is_empty());
            assert!(
                gate.claim_release().await.is_none(),
                "terminal state refuses future EXEC claims"
            );
        }
    }
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn illegal_event_from_every_gate_state_is_rejected() {
    let boot = GuestNetworkExecWiring::new(Arc::new(SimClock::new())).supervisor();
    assert!(!boot.begin_recovery(SharedGuestNetworkComponent::Forwarder));
    assert!(!boot.complete_attempt(BTreeSet::new()));

    let open = GuestNetworkExecWiring::new(Arc::new(SimClock::new())).supervisor();
    assert!(open.open_after_boot());
    assert!(!open.open_after_boot());
    assert!(!open.complete_attempt(BTreeSet::new()));

    let recovering = GuestNetworkExecWiring::new(Arc::new(SimClock::new())).supervisor();
    assert!(recovering.open_after_boot());
    assert!(recovering.begin_recovery(SharedGuestNetworkComponent::Forwarder));
    assert!(!recovering.open_after_boot());
    assert!(recovering.begin_recovery(SharedGuestNetworkComponent::Dns));
    assert!(!recovering.begin_recovery(SharedGuestNetworkComponent::Forwarder));

    let failed = GuestNetworkExecWiring::new(Arc::new(SimClock::new())).supervisor();
    assert!(failed.fail_stop(SharedGuestNetworkFailStopCause::SupervisorFailed).is_some());
    assert!(!failed.open_after_boot());
    assert!(!failed.begin_recovery(SharedGuestNetworkComponent::Forwarder));
    assert!(!failed.complete_attempt(BTreeSet::new()));
    assert!(failed.fail_stop(SharedGuestNetworkFailStopCause::SupervisorReturned).is_none());
}
