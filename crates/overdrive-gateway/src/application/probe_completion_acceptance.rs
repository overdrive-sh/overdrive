//! S-PIG-73: a connector probe result completed twice for one staged token is
//! ignored the second time (assumption GA-1).
//!
//! The real Application owner is composed over the held-gate doubles of
//! [`super::held_probe_doubles`]; the candidate's gate parks inside the connect
//! dataplane probe. The oracle is `GatewayStatusReads::application_status()`
//! and the number of times that probe returned. The runtime is paused and single-threaded, and the owner reads time
//! from the injected `SimClock`, so no wall-clock sleep takes part. The
//! composition binds `127.64.0.1:443` (R-6, table in `held_probe_doubles`).
#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use super::held_probe_doubles::{
    Composition, ProbeOutcome, ReleaseOutcome, route, settle, wait_until,
};

/// Second octet reserved for this scenario in the bind-address table.
const PROBE_COMPLETION_OCTET: u8 = 64;

/// S-PIG-73 (GA-1): a staged candidate's gate is released usable, the
/// generation is promoted, and the same held gate is then completed again
/// with a usable result and with an unusable one. The connect probe returned
/// exactly once, for the first completion; each second completion finds no
/// entry and returns nothing more. The status row is byte-equal before and
/// after, the Applied generation is the one the first completion promoted, and
/// nothing is staged or draining.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(start_paused = true)]
async fn a_probe_result_completed_twice_is_ignored_the_second_time() {
    let world = Composition::start(PROBE_COMPLETION_OCTET, 0).await;
    world.routes.routes().apply(route("/")).await.expect("declare the Route");

    // Exactly one probe is held, for the one staged candidate.
    let staged = wait_until(&world.status, &world.probes, "the candidate staged and held", {
        |status, probes| status.is_some_and(|row| row.staged.is_some()) && probes.held_count() == 1
    })
    .await
    .expect("candidate staged with one held probe")
    .expect("status row");
    assert!(staged.current.is_none(), "a parked probe promotes nothing");
    let held = world.probes.held_gates();
    assert_eq!(held.len(), 1, "exactly one entry is held");
    let gate = held[0];

    // First completion: usable. The generation is promoted.
    assert_eq!(world.probes.release(gate, ProbeOutcome::Usable), ReleaseOutcome::Released);
    let promoted = wait_until(&world.status, &world.probes, "the candidate promoted", {
        |status, _| status.is_some_and(|row| row.current.is_some() && row.staged.is_none())
    })
    .await
    .expect("candidate promoted")
    .expect("status row");
    let applied = promoted.current.as_ref().expect("Current generation").application_generation;
    assert_eq!(
        Some(applied),
        staged.staged.as_ref().map(|candidate| candidate.application_generation),
        "the promoted generation is the staged candidate",
    );
    assert_eq!(world.probes.held_count(), 0, "the released entry is no longer held");
    assert_eq!(world.probes.connect_returned(), 1, "the released connect probe returned once");

    // The owner is idle: the status row it published is the "before" snapshot.
    let before = settle(&world.status).await.expect("owner idle after promotion");

    // Second completion for the same identity, usable and unusable: a no-op.
    assert_eq!(
        world.probes.release(gate, ProbeOutcome::Usable),
        ReleaseOutcome::Absent,
        "a second usable completion finds no entry",
    );
    assert_eq!(
        world.probes.release(gate, ProbeOutcome::Unusable),
        ReleaseOutcome::Absent,
        "a second unusable completion finds no entry",
    );
    let after = settle(&world.status).await.expect("owner idle after the second completions");

    assert_eq!(after, before, "the status row is unchanged by the second completions");
    let row = after.expect("status row");
    assert_eq!(
        row.current.as_ref().map(|current| current.application_generation),
        Some(applied),
        "the Applied generation is unchanged",
    );
    assert!(row.staged.is_none(), "nothing is staged: {row:?}");
    assert!(row.draining.is_empty(), "nothing is draining: {row:?}");
    assert_eq!(world.probes.held_count(), 0, "no probe was re-parked");
    assert_eq!(
        world.probes.connect_returned(),
        1,
        "the second completions returned no connect probe",
    );

    world.stop().await;
}
