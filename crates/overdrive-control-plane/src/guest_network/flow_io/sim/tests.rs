//! The sim's own semantics (DG-3, QPROJ-7): holds, the node-global steering
//! state across seam instances, K-L3, the seam contract's `unreachable!`s.
//! These pin the fixture the owner tests and the `prefix_boot` driver stand
//! on; a sim that regressed would let a wrong owner pass silently.

#![allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};

use super::{
    SimEffectKind, SimFailure, SimGuestFlowIo, SimGuestNode, SimLanding, SimPrefixRoute,
    SimProgramVersion, SimSteerOutcome, SimSteeringEffect, SimSteeringFailure, SimSteeringLink,
};
use crate::guest_network::flow_io::{FlowListener, GuestFlowIo, OwnedSocket};

fn v(label: &str) -> SimProgramVersion {
    SimProgramVersion::new(label)
}

fn prefix() -> Ipv4Net {
    GUEST_NODE_PREFIX
}

fn intake(last: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, last), port)
}

/// Runs `converge_steering` on a task and steps it through the sim's holds,
/// releasing each held steering effect in turn (or failing the one named).
async fn boot_steering(
    io: &Arc<SimGuestFlowIo>,
    fail_at: Option<(SimEffectKind, SimSteeringFailure)>,
) -> Result<(), String> {
    let seam = Arc::clone(io);
    let task = tokio::spawn(async move {
        seam.converge_steering(prefix(), GUEST_NODE_GATEWAY).await.map_err(|e| e.to_string())
    });
    loop {
        io.settled().await;
        if task.is_finished() {
            return task.await.expect("converge_steering task did not panic");
        }
        let (id, effect) =
            *io.held().first().expect("converge_steering is parked on a held effect");
        match fail_at {
            Some((kind, failure)) if kind == effect.kind => {
                io.fail(id, SimFailure::Steering(failure));
            }
            _ => io.release(id),
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_boot_holds_load_then_attach_then_pin_and_pins_this_binarys_program() {
    let node = SimGuestNode::new();
    let io = SimGuestFlowIo::new(&node, v("v1"));
    let before = io.steering_observation();
    assert_eq!(before.link, SimSteeringLink::None);
    assert_eq!(before.route, SimPrefixRoute::Absent);

    boot_steering(&io, None).await.expect("steering converges");

    assert_eq!(
        io.steering_effects(),
        vec![
            SimSteeringEffect::Load { version: v("v1"), ok: true },
            SimSteeringEffect::Attach { adopted: false, ok: true },
            SimSteeringEffect::Pin { ok: true },
        ],
    );
    let after = io.steering_observation();
    assert_eq!(after.link, SimSteeringLink::Pinned);
    assert_eq!(after.program, Some(v("v1")));
    assert_eq!(after.route, SimPrefixRoute::Absent, "the steering never writes the route");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_later_boot_updates_the_adopted_link_without_a_pin_and_a_failed_update_keeps_the_earlier_program()
 {
    let node = SimGuestNode::new();
    let first = SimGuestFlowIo::new(&node, v("v1"));
    boot_steering(&first, None).await.expect("first boot converges");
    first.exit_process();
    assert_eq!(first.steering_observation().link, SimSteeringLink::Pinned, "pinned link survives");

    let failing = SimGuestFlowIo::new(&node, v("v2"));
    let refused =
        boot_steering(&failing, Some((SimEffectKind::SteeringAttach, SimSteeringFailure::Update)))
            .await;
    assert!(refused.is_err(), "the update failure is returned");
    assert_eq!(
        failing.steering_effects(),
        vec![
            SimSteeringEffect::Load { version: v("v2"), ok: true },
            SimSteeringEffect::Attach { adopted: true, ok: false },
        ],
    );
    assert_eq!(failing.steering_observation().program, Some(v("v1")), "K-L9: earlier program");
    failing.exit_process();

    let later = SimGuestFlowIo::new(&node, v("v2"));
    boot_steering(&later, None).await.expect("later boot converges");
    assert_eq!(
        later.steering_effects(),
        vec![
            SimSteeringEffect::Load { version: v("v2"), ok: true },
            SimSteeringEffect::Attach { adopted: true, ok: true },
        ],
        "an adopted link is updated and never pinned again",
    );
    let after = later.steering_observation();
    assert_eq!((after.link, after.program), (SimSteeringLink::Pinned, Some(v("v2"))));
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_load_failure_attaches_nothing_and_a_pin_failure_detaches_the_new_link() {
    let node = SimGuestNode::new();
    let rejected = SimGuestFlowIo::new(&node, v("v1"));
    let outcome =
        boot_steering(&rejected, Some((SimEffectKind::SteeringLoad, SimSteeringFailure::Load)))
            .await;
    assert!(outcome.is_err());
    assert_eq!(
        rejected.steering_effects(),
        vec![SimSteeringEffect::Load { version: v("v1"), ok: false }]
    );
    assert_eq!(rejected.steering_observation().link, SimSteeringLink::None);
    rejected.exit_process();

    let unpinned = SimGuestFlowIo::new(&node, v("v1"));
    let outcome =
        boot_steering(&unpinned, Some((SimEffectKind::SteeringPin, SimSteeringFailure::Pin))).await;
    assert!(outcome.is_err());
    assert_eq!(unpinned.steering_observation().link, SimSteeringLink::None, "detached on return");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_crash_between_a_first_attach_and_its_pin_leaves_no_link_while_a_pinned_link_and_the_route_survive()
 {
    let node = SimGuestNode::new();
    let io = SimGuestFlowIo::new(&node, v("v1"));
    let seam = Arc::clone(&io);
    let task =
        tokio::spawn(async move { seam.converge_steering(prefix(), GUEST_NODE_GATEWAY).await });
    io.settled().await;
    io.release(io.held_of(SimEffectKind::SteeringLoad).expect("load held"));
    io.settled().await;
    io.release(io.held_of(SimEffectKind::SteeringAttach).expect("attach held"));
    io.settled().await;
    assert!(io.held_of(SimEffectKind::SteeringPin).is_some(), "parked on the pin");
    assert_eq!(io.steering_observation().link, SimSteeringLink::Attached);

    task.abort();
    let _cancelled = task.await;
    io.exit_process();
    assert_eq!(io.steering_observation().link, SimSteeringLink::None, "unpinned link died");

    let next = SimGuestFlowIo::new(&node, v("v1"));
    boot_steering(&next, None).await.expect("next boot converges");
    let seam = Arc::clone(&next);
    let route = tokio::spawn(async move {
        seam.converge_prefix_route(prefix(), GUEST_NODE_GATEWAY).await.map_err(|e| e.to_string())
    });
    next.settled().await;
    next.release(next.held_of(SimEffectKind::PrefixRoute).expect("route write held"));
    next.settled().await;
    route.await.expect("route task").expect("route converges");
    next.exit_process();
    let after = next.steering_observation();
    assert_eq!((after.link, after.route), (SimSteeringLink::Pinned, SimPrefixRoute::Local));
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[should_panic(expected = "before converge_steering returned Ok")]
async fn the_route_write_before_the_steering_converged_is_unreachable() {
    let node = SimGuestNode::new();
    let io = SimGuestFlowIo::new(&node, v("v1"));
    io.auto_release_all();
    let _unreachable = io.converge_prefix_route(prefix(), GUEST_NODE_GATEWAY).await;
}

async fn booted_seam(node: &Arc<SimGuestNode>) -> Arc<SimGuestFlowIo> {
    let io = SimGuestFlowIo::new(node, v("v1"));
    io.auto_release_all();
    io.load_forwarder(&BTreeSet::new()).await.expect("forwarder loads");
    io.converge_steering(prefix(), GUEST_NODE_GATEWAY).await.expect("steering converges");
    io
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn closing_an_intake_listener_removes_its_steering_entry_in_the_same_step() {
    let node = SimGuestNode::new();
    let io = booted_seam(&node).await;
    let declared = intake(2, 8080);
    let listener = io.bind_intake(intake(2, 20_480)).await.expect("intake bound");
    io.steer(declared, &listener).await.expect("steered");
    assert!(io.steering_observation().entries.contains_key(&declared));
    assert!(matches!(
        io.connect_intake_client(declared, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 50_000)),
        SimLanding::Steered(_)
    ));

    io.close(OwnedSocket::from(listener)).await;

    assert!(io.steering_observation().entries.is_empty(), "K-L3: entry gone with the listener");
    assert_eq!(
        io.connect_intake_client(declared, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 50_001)),
        SimLanding::Refused,
    );
    assert_eq!(
        io.steering_effects().last(),
        Some(&SimSteeringEffect::Steer { intake: declared, outcome: SimSteerOutcome::Ok })
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn steer_classifies_kernel_memory_and_a_second_listener_on_one_intake() {
    let node = SimGuestNode::new();
    let io = booted_seam(&node).await;
    let declared = intake(3, 9000);
    let first = io.bind_intake(intake(3, 20_480)).await.expect("bound");
    io.script_failure(SimEffectKind::Steer, SimFailure::Steering(SimSteeringFailure::KernelMemory));
    assert!(io.steer(declared, &first).await.is_err());
    assert!(io.steering_observation().entries.is_empty(), "KernelMemory: entry absent");
    io.steer(declared, &first).await.expect("retry steers");
    io.steer(declared, &first).await.expect("repeat for the same listener is Ok");
    let second = io.bind_intake(intake(3, 20_481)).await.expect("bound");
    assert!(io.steer(declared, &second).await.is_err(), "IntakeAlreadySteered");
    let outcomes: Vec<SimSteerOutcome> = io
        .steering_effects()
        .into_iter()
        .filter_map(|effect| match effect {
            SimSteeringEffect::Steer { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(
        outcomes,
        vec![
            SimSteerOutcome::KernelMemory,
            SimSteerOutcome::Ok,
            SimSteerOutcome::Ok,
            SimSteerOutcome::Defect,
        ]
    );
    io.close(OwnedSocket::from(first)).await;
    io.close(OwnedSocket::from(second)).await;
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_held_effect_dropped_by_the_owner_never_applies() {
    let node = SimGuestNode::new();
    let io = SimGuestFlowIo::new(&node, v("v1"));
    let seam = Arc::clone(&io);
    let task = tokio::spawn(async move { seam.bind_listener(FlowListener::Control).await });
    io.settled().await;
    assert!(io.held_of(SimEffectKind::BindListener).is_some());
    task.abort();
    let _cancelled = task.await;
    io.settled().await;
    assert!(io.held().is_empty(), "the dropped call's hold is gone");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_second_load_forwarder_returns_the_earlier_convergence_and_host_facts_are_scriptable() {
    let node = SimGuestNode::new();
    let io = SimGuestFlowIo::new(&node, v("v1"));
    io.auto_release_all();
    let first = io.load_forwarder(&BTreeSet::new()).await.expect("loads");
    let second = io.load_forwarder(&BTreeSet::new()).await.expect("returns the earlier one");
    assert_eq!((first.attached, second.attached), (BTreeSet::new(), BTreeSet::new()));

    let facts = io.host_facts().await.expect("default facts");
    assert_eq!(facts.ephemeral_ports, 32_768..=60_999);
    node.update_host_facts(|facts| facts.ephemeral_ports = 20_000..=60_999);
    assert_eq!(io.host_facts().await.expect("updated").ephemeral_ports, 20_000..=60_999);
    node.fail_host_facts(std::io::ErrorKind::PermissionDenied);
    assert!(io.host_facts().await.is_err(), "a fact is unreadable: never a default");
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[should_panic(expected = "another seam instance")]
async fn a_handle_of_another_seam_instance_is_unreachable() {
    let node = SimGuestNode::new();
    let one = booted_seam(&node).await;
    let other = SimGuestFlowIo::new(&node, v("v1"));
    other.auto_release_all();
    let listener = one.bind_intake(intake(4, 20_480)).await.expect("bound");
    other.close(OwnedSocket::from(listener)).await;
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "owner defect")]
fn a_handle_left_open_when_the_seam_drops_without_a_process_exit_is_reported() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
    runtime.block_on(async {
        let node = SimGuestNode::new();
        let io = booted_seam(&node).await;
        let listener = io.bind_intake(intake(5, 20_480)).await.expect("bound");
        drop(listener);
        drop(io);
    });
}
