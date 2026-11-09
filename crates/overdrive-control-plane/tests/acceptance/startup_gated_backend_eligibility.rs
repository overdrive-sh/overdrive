//! A startup-probed allocation is a backend only after its own `Stable` is
//! committed (feature `service-allocation-readiness-and-ending`: D12, gate
//! G-9; obligation S-47 of `feature-delta.md` § Executable obligations for
//! DISTILL).
//!
//! Composition: `restart_authority_world`. The oracle is the written
//! `ServiceBackendRow` and the mesh resolver's `BackendIndex` read of it
//! (`BackendIndex::apply_row` then `classify`): `Mesh` routes to the backend,
//! `MeshUnreachable` is the existing fail-closed result. Tier 1 composed,
//! seeded (seed printed). A dropped row write is repaired by the next
//! evaluation, so every scenario looks twice around the `Stable` commit.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::doc_markdown,
    reason = "scenario bodies are linear; the CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use overdrive_core::id::AllocationId;

use crate::acceptance::restart_authority_world::{Dial, RestartWorld, Shape};

/// The backend is not routable: no row, or a row that marks it unhealthy.
async fn not_a_backend(world: &RestartWorld, alloc: &AllocationId) -> bool {
    world.backend_healthy(alloc).await != Some(true)
        && world.resolver_routes_to(alloc).await != Some(true)
}

/// Two looks around the `Stable` commit (the commit, then the evaluation that
/// reads it).
async fn stable_commits(world: &mut RestartWorld) {
    world.startup_period(Dial::Binds).await;
    world.service_tick().await.expect("look announcing Stable");
    world.service_tick().await.expect("look reading the committed Stable");
}

/// S-47 (a): first allocation `Running`, startup probe unbound then passing:
/// not a backend from `Running` until the `Stable` row is committed, then a
/// backend; the resolver fails closed before and routes to it after.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D12 a startup-probed allocation is a backend only after its own Stable is committed"]
async fn startup_probed_allocation_is_a_backend_only_after_its_stable_is_committed() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let alloc = world.place_first_attempt().await;
    world.service_tick().await.expect("look at the Running row");
    world.startup_period(Dial::Refuses).await;
    world.service_tick().await.expect("look while the startup probe fails");
    assert!(not_a_backend(&world, &alloc).await, "not a backend while unbound");

    stable_commits(&mut world).await;
    assert!(format!("{:?}", world.row(&alloc).await.expect("row").terminal).contains("Stable"));
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));
    assert_eq!(world.resolver_routes_to(&alloc).await, Some(true));
}

/// S-47 (b): the first attempt is startup-stopped, the successor is `Running`:
/// the successor is not a backend until its own `Stable` is committed.
#[tokio::test]
async fn successor_of_a_startup_stopped_attempt_is_a_backend_only_after_its_own_stable() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.never_ready_until_the_look(2).await.expect("the first attempt is stopped");
    assert!(not_a_backend(&world, &first).await);
    let successor = world.restart_successor().await;
    world.service_tick().await.expect("look at the successor");
    assert!(not_a_backend(&world, &successor).await, "the successor has no committed Stable yet");

    stable_commits(&mut world).await;
    assert_eq!(world.resolver_routes_to(&successor).await, Some(true));
    assert!(not_a_backend(&world, &first).await, "the stopped predecessor is never a backend");
}

/// S-47 (e): after `Stable`, an extra startup `Fail` row lands (the startup role
/// is not yet retired): the backend stays healthy.
#[tokio::test]
async fn startup_failure_after_stable_does_not_make_the_backend_unhealthy() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let alloc = world.place_first_attempt().await;
    stable_commits(&mut world).await;
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));

    world.startup_period(Dial::Refuses).await;
    world.service_tick().await.expect("look after the extra startup Fail");
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));
    assert_eq!(world.resolver_routes_to(&alloc).await, Some(true));
}

/// S-47 (f): a control-plane restart (`View` reloaded) between `Stable` and the
/// next evaluation re-derives eligibility from the row, with no view marker.
#[tokio::test]
async fn control_plane_restart_after_stable_keeps_the_backend_healthy() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let alloc = world.place_first_attempt().await;
    stable_commits(&mut world).await;
    world.restart_control_plane().await;
    world.service_tick().await.expect("look after the control-plane restart");
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));
    assert_eq!(world.resolver_routes_to(&alloc).await, Some(true));
}

/// S-47 (g): a crash successor (D6) is a backend only after its own `Stable`.
#[tokio::test]
async fn crash_successor_is_a_backend_only_after_its_own_stable() {
    let mut world = RestartWorld::compose(Shape::StartupProbe).await;
    let first = world.place_first_attempt().await;
    world.guest_exits(&first, Some(1), None).await;
    world.service_tick().await.expect("look at the crashed row");
    let successor = world.restart_successor().await;
    world.service_tick().await.expect("look at the successor");
    assert!(not_a_backend(&world, &successor).await);

    stable_commits(&mut world).await;
    assert_eq!(world.resolver_routes_to(&successor).await, Some(true));
}

/// S-47 (h): an opt-out Service is a backend at `Running`, as today.
#[tokio::test]
async fn opt_out_service_is_a_backend_at_running() {
    let mut world = RestartWorld::compose(Shape::OptOut).await;
    let alloc = world.place_first_attempt().await;
    world.service_tick().await.expect("look at the Running row");
    world.service_tick().await.expect("second look");
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));
    assert_eq!(world.resolver_routes_to(&alloc).await, Some(true));
}

/// S-47 (i): a readiness-probed Service whose readiness streak met the
/// threshold before `Stable` is eligible at the `Stable` commit, not before.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D12 the readiness rule only narrows a committed Stable"]
async fn readiness_met_before_stable_makes_the_backend_eligible_at_the_stable_commit() {
    let mut world = RestartWorld::compose(Shape::StartupAndReadiness).await;
    let alloc = world.place_first_attempt().await;
    world.readiness_passes().await;
    world.service_tick().await.expect("look with readiness met and startup unbound");
    assert!(not_a_backend(&world, &alloc).await, "readiness alone does not make a backend");

    stable_commits(&mut world).await;
    assert_eq!(world.backend_healthy(&alloc).await, Some(true));
    assert_eq!(world.resolver_routes_to(&alloc).await, Some(true));
}
