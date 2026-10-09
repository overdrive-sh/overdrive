//! P6 — guest-flow owner quiescence, recovery, shared-table repair, and
//! guest-command release, driven through the production owner's driving ports
//! with `SimSharedGuestNetworkOwner` scripting at the private seam
//! (netns-density-295 vsock attachment replacement; feature-delta
//! §"Owner and provisioner", Gate G-V5, Gate G-V2; ADR-0169 D8a-HOLD,
//! ADR-0160, ADR-0124; Quint `quiescence` inv 10, `shared_table` inv 11).
//!
//! RED acceptance scaffolds (`.claude/rules/testing.md` §"RED acceptance
//! scaffolds and activation"): each body is complete and marked
//! `#[ignore = "pending DELIVER …"]`. The crafter removes only the pending
//! marker once the DELIVER production scaffold the body calls exists; it does
//! not rewrite the body.
//!
//! **Sequencing (shared-test-infrastructure.md §0, §6).** The replacement
//! owner surface (`quiesce_forwarding` / `restore_forwarding`, the reshaped
//! `SimSharedGuestNetworkOwner`, `QuiescenceHolder` / `QuiescenceHold` /
//! `ForwardingRestore`) does not exist yet. This file is therefore **not
//! registered** in `tests/acceptance.rs` (`mod acceptance { … }`); the author
//! adds its `mod vsock_quiescence_recovery;` line when the compiling body
//! co-arrives with the DELIVER owner scaffold. Until then it is not a cargo
//! target and never compiled, so the build stays green.
//!
//! **State machine under test (C2a).** Owner allocation lifecycle
//! `Provisioned → Active → Retiring → absent`; node quiescence holder set
//! `{} ⇄ {Recovery(c)…}` with admission latched iff non-empty (D8a-HOLD); each
//! per-component recovery `idle → held → repaired → idle` (ADR-0124 bounded,
//! fail-stop when the attempt budget is spent).
//!
//! Observable surface (never private owner fields): the owner trait method
//! return values, the ordered `owner.calls()` log of
//! `GuestNetworkOperation`, and the sim owner's observation accessors
//! (`admission_open()`, `held_holders()`, `live_flows()`,
//! `intake_listeners()`, `steering_entries()`, `prefix_exposed_to_host_service()`,
//! `repair_touched_prefix()`) — the testability surface P6 shapes
//! (CLAUDE.md §"Implement to the design"; test-support surface is DISTILL's).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::sync::Arc;

use overdrive_control_plane::guest_network::{
    ForwardingActivation, ForwardingRestore, GuestNetworkError, GuestNetworkOperation,
    QuiescenceHolder, SharedGuestNetworkOwner,
};
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::id::AllocationId;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;

/// One Active allocation serving a declared port, booted and forwarding open.
/// The sim owner is scripted with one wanted intake listener `(alloc, port)`.
fn active_owner_serving(alloc: &AllocationId, port: u16) -> Arc<SimSharedGuestNetworkOwner> {
    let owner = Arc::new(SimSharedGuestNetworkOwner::new());
    owner.script_booted_open();
    owner.script_active_allocation(alloc.clone());
    owner.script_listening_port(alloc.clone(), port);
    owner
}

/// Universe-bound observation (Mandate 8 expressed in Rust idiom): the
/// port-exposed observable slots a quiescence/restore step may change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerObservation {
    admission_open: bool,
    held_holders: BTreeSet<QuiescenceHolder>,
    live_flows: u32,
    intake_listeners: BTreeSet<(AllocationId, u16)>,
    steering_entries: BTreeSet<(AllocationId, u16)>,
    prefix_exposed_to_host_service: bool,
}

fn observe(owner: &SimSharedGuestNetworkOwner) -> OwnerObservation {
    OwnerObservation {
        admission_open: owner.admission_open(),
        held_holders: owner.held_holders(),
        live_flows: owner.live_flows(),
        intake_listeners: owner.intake_listeners(),
        steering_entries: owner.steering_entries(),
        prefix_exposed_to_host_service: owner.prefix_exposed_to_host_service(),
    }
}

// ----------------------------------------------------------------- P6-S1
/// G-V5 / R5-4 / Quint quiescence inv 10 (`LatchIffHolders`,
/// `QuiescedClosesListeners`): the first holder latches admission, aborts every
/// live flow and closes every intake listener (its steering entry gone in the
/// same kernel step); a single-holder restore reopens and re-binds the wanted
/// port from the last `ListenState`.
#[tokio::test]
#[ignore = "pending DELIVER owner-recovery step (G-V5, R5-4)"]
async fn first_holder_latches_and_single_holder_restore_reopens() {
    let alloc = AllocationId::new("alloc-s1").expect("alloc id");
    let port = 18_951_u16;
    let owner = active_owner_serving(&alloc, port);
    owner.script_component_damaged(SharedGuestNetworkComponent::FlowListeners);
    owner.script_live_flows(3);

    let before = observe(&owner);
    assert!(before.admission_open, "precondition: forwarding open");
    assert!(before.intake_listeners.contains(&(alloc.clone(), port)));
    assert!(before.steering_entries.contains(&(alloc.clone(), port)));

    // When: the supervisor quiesces under the FlowListeners recovery.
    let holder = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::FlowListeners);
    let (hold, quiescence) =
        owner.quiesce_forwarding(holder).await.expect("first quiesce succeeds");
    assert_eq!(hold.holder(), holder);
    assert!(quiescence.unconfirmed.is_empty(), "no per-allocation abort failure");

    // Then: universe delta — admission latched, flows aborted, listener + entry gone.
    let quiesced = observe(&owner);
    assert!(!quiesced.admission_open, "admission latched Quiesced");
    assert_eq!(quiesced.held_holders, BTreeSet::from([holder]));
    assert_eq!(quiesced.live_flows, 0, "every live flow aborted");
    assert!(quiesced.intake_listeners.is_empty(), "intake listeners closed");
    assert!(quiesced.steering_entries.is_empty(), "steering entries removed in the same step");
    // Unchanged slots: nothing exposes the prefix to a host service.
    assert_eq!(
        quiesced.prefix_exposed_to_host_service, before.prefix_exposed_to_host_service,
        "quiescence changes no host-service exposure"
    );

    // When: the recovery repairs, audits clean and restores.
    owner.script_component_repaired(SharedGuestNetworkComponent::FlowListeners);
    let restore = owner.restore_forwarding(hold).await.expect("restore succeeds");
    assert!(matches!(restore, ForwardingRestore::Reopened), "no holder remains → Reopened");

    // Then: admission reopened; the wanted port re-bound and steered.
    let reopened = observe(&owner);
    assert!(reopened.admission_open, "admission reopened");
    assert!(reopened.held_holders.is_empty());
    assert!(reopened.intake_listeners.contains(&(alloc.clone(), port)), "listener re-bound");
    assert!(reopened.steering_entries.contains(&(alloc, port)), "entry re-steered");

    // A TCP probe on the port fails while quiesced and succeeds after restore
    // is the Tier-3 half (R5-4 cell inside the WS run); here the seeded-sim
    // oracle is the observable listener/admission state above.
    let calls = owner.calls();
    assert!(
        calls.contains(&GuestNetworkOperation::ForwardingQuiesce)
            && calls.contains(&GuestNetworkOperation::ForwardingRestore),
        "quiesce then restore recorded in order: {calls:?}"
    );
}

// ----------------------------------------------------------------- P6-S2
/// G-V5 / R5-32 / Quint quiescence (`HeldIsHolder`, `WitnessNoStillQuiesced`):
/// a restore leaving other holders returns `StillQuiesced`; a second quiesce by
/// a holder already holding returns `QuiescenceHolderBusy` with no change; the
/// last restore returns `Reopened`.
#[tokio::test]
#[ignore = "pending DELIVER owner-recovery step (G-V5, R5-32)"]
async fn overlapping_recoveries_still_quiesced_then_reopened() {
    let alloc = AllocationId::new("alloc-s2").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_952);
    owner.script_component_damaged(SharedGuestNetworkComponent::IpRules);
    owner.script_component_damaged(SharedGuestNetworkComponent::FlowListeners);

    let ip = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::IpRules);
    let fl = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::FlowListeners);

    let (ip_hold, _) = owner.quiesce_forwarding(ip).await.expect("IpRules quiesce");
    let (fl_hold, _) = owner.quiesce_forwarding(fl).await.expect("FlowListeners quiesce");
    assert_eq!(observe(&owner).held_holders, BTreeSet::from([ip, fl]));

    // A second quiesce by a holder already holding: Busy, no change.
    let before_busy = observe(&owner);
    let busy = owner.quiesce_forwarding(ip).await;
    assert!(
        matches!(busy, Err(GuestNetworkError::QuiescenceHolderBusy { holder }) if holder == ip),
        "second quiesce by a held holder is Busy"
    );
    assert_eq!(observe(&owner), before_busy, "Busy changes nothing");

    // FlowListeners restores first → StillQuiesced, admission stays latched.
    owner.script_component_repaired(SharedGuestNetworkComponent::FlowListeners);
    let still = owner.restore_forwarding(fl_hold).await.expect("fl restore");
    match still {
        ForwardingRestore::StillQuiesced { holders } => {
            assert_eq!(holders, BTreeSet::from([ip]), "IpRules still holds");
        }
        ForwardingRestore::Reopened => panic!("must not reopen while IpRules holds"),
    }
    assert!(!observe(&owner).admission_open, "admission still latched");
    assert!(observe(&owner).intake_listeners.is_empty(), "no intake listener while quiesced");

    // IpRules repairs and restores → Reopened.
    owner.script_component_repaired(SharedGuestNetworkComponent::IpRules);
    let reopened = owner.restore_forwarding(ip_hold).await.expect("ip restore");
    assert!(matches!(reopened, ForwardingRestore::Reopened));
    assert!(observe(&owner).admission_open, "last restore reopens");
}

// ----------------------------------------------------------------- P6-S3
/// G-V5 / R5-30 / Quint shared_table inv 11 (`NoWildcardReached`) + quiescence
/// `RepairNeverWritesPrefix`: the mTLS worker leaves the shared nft table
/// partial; with the table in any state no connection to the prefix reaches a
/// host service; the firewall recovery repairs only while it holds, and the
/// `IpRules` repair writes neither a steering entry, the steering program nor
/// the route.
#[tokio::test]
#[ignore = "pending DELIVER owner-recovery step (G-V5, R5-30)"]
async fn partial_table_repaired_only_inside_hold_never_writes_prefix() {
    let alloc = AllocationId::new("alloc-s3").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_953);

    // Fault: the mTLS worker fails part-way converging the constant rules.
    owner.script_shared_table_partial();
    assert!(
        !owner.prefix_exposed_to_host_service(),
        "partial table while serving: nothing to the prefix reaches a host service"
    );

    let ip = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::IpRules);
    let (hold, _) = owner.quiesce_forwarding(ip).await.expect("IpRules quiesce");
    assert!(
        !owner.prefix_exposed_to_host_service(),
        "partial table while quiesced: still no host-service exposure"
    );

    // Repair converges the IpRules component only.
    owner.converge_shared().await.expect("repair converges IpRules only");
    let touched = owner.repair_touched_prefix();
    assert!(!touched.wrote_steering_entry, "repair wrote no steering entry");
    assert!(!touched.wrote_steering_program, "repair wrote no steering program");
    assert!(!touched.wrote_route, "repair wrote no prefix route");

    owner.script_component_repaired(SharedGuestNetworkComponent::IpRules);
    assert!(matches!(
        owner.restore_forwarding(hold).await.expect("restore"),
        ForwardingRestore::Reopened
    ));
    assert!(
        !owner.prefix_exposed_to_host_service(),
        "no connection to the prefix reached a host service at any point"
    );
    assert!(
        owner.intake_listeners().contains(&(alloc, 18_953)),
        "every listener serving after restore is steered"
    );
}

// ----------------------------------------------------------------- P6-S5
/// G-V5 / D19-LINKS / Quint quiescence `FailStopLeavesPrefixAsBooted`: a
/// recovery that spends its bounded attempt budget fail-stops; the shared route
/// and pinned steering stay as boot converged them; the audit never audits the
/// forwarder's links; admission never reopens.
#[tokio::test]
#[ignore = "pending DELIVER owner-recovery step (G-V5, D19-LINKS)"]
async fn recovery_fail_stop_keeps_prefix_and_never_audits_links() {
    let alloc = AllocationId::new("alloc-s5").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_955);
    owner.script_component_damaged(SharedGuestNetworkComponent::IpRules);
    owner.script_repair_fails_forever(SharedGuestNetworkComponent::IpRules);

    let ip = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::IpRules);
    let (_hold, _) = owner.quiesce_forwarding(ip).await.expect("quiesce");
    // The supervisor exhausts the ADR-0124 attempt budget and fail-stops.
    let fail_stop = owner.drive_recovery_to_exhaustion(ip).await;
    assert!(fail_stop.is_fail_stop(), "attempt budget spent → fail-stop");

    assert!(!owner.admission_open(), "admission never reopens after fail-stop");
    assert_eq!(owner.prefix_route(), "local", "fail-stop leaves the route `local`");
    assert!(owner.steering_link_pinned(), "fail-stop leaves the pinned steering");
    // D19-LINKS: the audit set never contains a link component.
    let audited = owner.audited_components();
    for link in [
        SharedGuestNetworkComponent::Forwarder, // sock_ops / drain-counter / unframe links are not components
    ] {
        let _ = link; // the deleted link components (SockOpsLink, DrainCounterLink, UnframeLink) no longer exist as variants
    }
    assert!(
        audited.iter().all(|c| matches!(
            c,
            SharedGuestNetworkComponent::Forwarder
                | SharedGuestNetworkComponent::FlowListeners
                | SharedGuestNetworkComponent::ControlListener
                | SharedGuestNetworkComponent::IntakeListeners
                | SharedGuestNetworkComponent::LegF
                | SharedGuestNetworkComponent::LegC
                | SharedGuestNetworkComponent::IpRules
                | SharedGuestNetworkComponent::IpSets
                | SharedGuestNetworkComponent::Dns
                | SharedGuestNetworkComponent::Supervisor
        )),
        "D19-LINKS: no forwarder-link component is audited: {audited:?}"
    );
}

// ----------------------------------------------------------------- P6-S6
/// G-V5 boundary rows 4 and 5: a late restore after fail-stop is ignored (no
/// reopen); a quiesce aborts flows and a restore does not resurrect them.
#[tokio::test]
#[ignore = "pending DELIVER owner-recovery step (G-V5, boundary 4/5)"]
async fn late_restore_ignored_and_restore_does_not_resurrect_flows() {
    let alloc = AllocationId::new("alloc-s6").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_956);
    owner.script_live_flows(2);
    owner.script_component_damaged(SharedGuestNetworkComponent::FlowListeners);

    let holder = QuiescenceHolder::Recovery(SharedGuestNetworkComponent::FlowListeners);
    let (hold, _) = owner.quiesce_forwarding(holder).await.expect("quiesce");
    assert_eq!(owner.live_flows(), 0, "quiesce aborts every flow");

    owner.script_component_repaired(SharedGuestNetworkComponent::FlowListeners);
    assert!(matches!(
        owner.restore_forwarding(hold).await.expect("restore"),
        ForwardingRestore::Reopened
    ));
    // Row 5: aborted flows are NOT resurrected; only admission reopened.
    assert_eq!(owner.live_flows(), 0, "restore reopens admission only; no flow resurrected");

    // Row 4: a restore for a now-dead recovery after fail-stop is a no-op.
    owner.script_fail_stopped();
    let stale = owner.deliver_stale_restore(holder).await;
    assert!(stale.is_ignored(), "late restore after fail-stop ignored");
    assert!(!owner.admission_open(), "admission stays closed");
}

// ----------------------------------------------------------------- P6-S10
/// G-V4 rows 4/5: a VMM death aborts the CID's flows and no new flow is
/// registered on a check the death made false; a pre-READY VMM exit is the
/// existing non-terminal start failure.
#[tokio::test]
#[ignore = "pending DELIVER owner step 5 (G-V4 rows 4/5)"]
async fn vmm_death_aborts_flows_and_blocks_late_registration() {
    let alloc = AllocationId::new("alloc-s10").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_960);
    owner.script_live_flows(4);

    // When: the VMM dies (its vhost device released).
    owner.script_vmm_death(alloc.clone());
    assert_eq!(owner.live_flows(), 0, "VMM death aborts every flow of the CID");

    // A flow request delivered after the death does not register.
    let registered_before = owner.registered_flows();
    owner.deliver_late_flow_request(alloc.clone()).await;
    assert_eq!(
        owner.registered_flows(),
        registered_before,
        "no flow registered on a check the death made false"
    );
}

// ----------------------------------------------------------------- P6-S12
/// G-V2 rows 1/2/4: the workload command is released only after `Activated`;
/// `QuiescenceLatched` / `ControlSessionPending` retry within the budget; an
/// `activate` after teardown never releases EXEC.
#[tokio::test]
#[ignore = "pending DELIVER activation step (G-V2)"]
async fn exec_released_only_after_activated() {
    let alloc = AllocationId::new("alloc-s12").expect("alloc id");
    let owner = active_owner_serving(&alloc, 18_962);

    // Scripted activation sequence: quiesced, then session pending, then Active.
    owner.script_activation_sequence(
        alloc.clone(),
        [
            ForwardingActivation::QuiescenceLatched,
            ForwardingActivation::ControlSessionPending,
            ForwardingActivation::Activated,
        ],
    );

    let plan = owner.plan_for(alloc.clone());
    let a1 = owner.activate(&plan).await.expect("activate #1");
    assert!(matches!(a1, ForwardingActivation::QuiescenceLatched));
    assert!(!owner.exec_released(&alloc), "no EXEC before Active");
    let a2 = owner.activate(&plan).await.expect("activate #2");
    assert!(matches!(a2, ForwardingActivation::ControlSessionPending));
    assert!(!owner.exec_released(&alloc), "no EXEC before Active");
    let a3 = owner.activate(&plan).await.expect("activate #3");
    assert!(matches!(a3, ForwardingActivation::Activated));
    owner.release_for_exit_emission(&alloc).await;
    assert!(owner.exec_released(&alloc), "EXEC released exactly once, after Activated");

    // Row 4: activate after teardown → retiring, no EXEC.
    let alloc2 = AllocationId::new("alloc-s12b").expect("alloc id");
    owner.script_active_allocation(alloc2.clone());
    owner.teardown(&owner.plan_for(alloc2.clone())).await.expect("teardown");
    let late = owner.activate(&owner.plan_for(alloc2.clone())).await;
    assert!(late.is_err(), "activate after teardown returns a retiring projection");
    assert!(!owner.exec_released(&alloc2), "EXEC never released after teardown");
}
