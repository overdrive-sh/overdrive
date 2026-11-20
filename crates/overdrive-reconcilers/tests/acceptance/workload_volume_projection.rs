//! Node-local persistent volume (GH #348, ADR-0310, ADR-0311): the projection from a
//! workload's `[vm.volume]` declaration to the `VolumePayload` every launch carries.
//!
//! The one projection `Vm::to_payload` is consumed at the three `WorkloadLifecycle` sites
//! that build a `VmPayload` from the intent: the start action, the restart action and
//! `allocation_spec_for_live_intent`. Driven through the existing driving ports
//! the production wire spec `JobSpecInput` and `Job::from_submit` (admission),
//! `WorkloadLifecycle::reconcile` (Start and Restart actions) and
//! `workload_lifecycle::allocation_spec_for_live_intent`. No `Vm` field is named: the volume
//! reaches the intent through the wire spec, so the test fails today where the wire spec
//! refuses `vm.volume` and needs no unpinned API.
//!
//! Evidence tier: in-memory acceptance (layer 2).

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, JobSpecInput, Node, WorkloadIntent, WorkloadKind};
use overdrive_core::guest_network::GuestAttachmentOccupancy;
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::GuestAttachmentObservation;
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::workload_lifecycle::allocation_spec_for_live_intent;
use overdrive_reconcilers::{WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView};
use serde_json::json;

const WORKLOAD: &str = "dev-box";
fn wid() -> WorkloadId {
    WorkloadId::new(WORKLOAD).expect("valid workload id")
}

fn nid() -> NodeId {
    NodeId::new("local").expect("valid node id")
}

/// The job a `[vm.volume]` declaration admits to, through the production wire shape and the
/// validating constructor `submit_workload` uses.
fn volume_job() -> Job {
    let spec: JobSpecInput = serde_json::from_value(json!({
        "id": WORKLOAD,
        "replicas": 1,
        "resources": {"cpu_milli": 500, "memory_bytes": 134_217_728u64},
        "vm": {
            "command": "/sbin/init", "args": [], "kernel": "/k", "rootfs": "/r",
            "volume": {"size": "40GiB"}
        }
    }))
    .expect("a [vm.volume] workload must be admitted by the production wire spec");
    Job::from_submit(spec).expect("a one-replica volume workload is valid")
}

fn state(
    job: &Job,
    kind: WorkloadKind,
    allocations: BTreeMap<AllocationId, AllocStatusRow>,
) -> WorkloadLifecycleState {
    WorkloadLifecycleState {
        current_startup_probe: None,
        workload_id: job.id.clone(),
        job: Some(job.clone()),
        desired_to_stop: false,
        generation: 0,
        nodes: BTreeMap::from([(
            nid(),
            Node {
                id: nid(),
                region: Region::new("local").expect("valid region"),
                capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
            },
        )]),
        allocations,
        workload_kind: kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        declared_listeners: Vec::new(),
        guest_attachments: GuestAttachmentObservation {
            occupancy: GuestAttachmentOccupancy { held: 0, retiring: 0 },
            leases: BTreeMap::new(),
        },
    }
}

/// A crashed `Service` allocation: the only kind a crash restarts. A `Job`'s `Failed` row is a
/// natural exit the reconciler finalizes rather than restarts.
fn failed_predecessor() -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new("alloc-dev-box-0").expect("valid allocation id"),
        workload_id: wid(),
        node_id: nid(),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 1, writer: nid() },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(1),
            signal: None,
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(10))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn reconcile(job: &Job, kind: WorkloadKind, rows: Vec<AllocStatusRow>) -> Vec<Action> {
    let allocations = rows.into_iter().map(|row| (row.alloc_id.clone(), row)).collect();
    let desired = state(job, kind, BTreeMap::new());
    let actual = state(job, kind, allocations);
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(20)),
        tick: 20,
        deadline: now + Duration::from_secs(1),
    };
    WorkloadLifecycle::canonical()
        .reconcile(&desired, &actual, &WorkloadLifecycleView::default(), &tick)
        .0
}

/// The volume the payload of `spec` carries, as `(workload, size as the operator writes it)`.
fn payload_volume(spec: &AllocationSpec) -> Option<(WorkloadId, String)> {
    match &spec.driver {
        DriverPayload::Vm(vm) => {
            vm.volume.as_ref().map(|v| (v.workload.clone(), v.size.to_string()))
        }
    }
}

fn assert_carries_volume(site: &str, spec: &AllocationSpec) {
    assert_eq!(
        payload_volume(spec),
        Some((wid(), "40GiB".to_owned())),
        "{site} must carry the declared volume in its VmPayload"
    );
}

/// The start action carries the workload's volume.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_start_action_carries_the_declared_volume() {
    let job = volume_job();
    let actions = reconcile(&job, WorkloadKind::Job, Vec::new());
    let spec = actions
        .iter()
        .find_map(|action| match action {
            Action::StartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("a workload with no allocation is started");
    assert_carries_volume("the start action", spec);
}

/// The restart action carries the workload's volume.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_restart_action_carries_the_declared_volume() {
    let job = volume_job();
    let actions = reconcile(&job, WorkloadKind::Service, vec![failed_predecessor()]);
    let spec = actions
        .iter()
        .find_map(|action| match action {
            Action::RestartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("a crashed single-replica service allocation is replaced by a restart");
    assert_carries_volume("the restart action", spec);
}

/// The spec reconstructed for a live allocation (control-plane boot recovery) carries the
/// workload's volume: the third projection site cannot diverge from the first two.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_spec_rebuilt_for_a_live_allocation_carries_the_declared_volume() {
    let job = volume_job();
    let alloc = AllocationId::new("alloc-dev-box-0").expect("valid allocation id");
    let spec = allocation_spec_for_live_intent(&WorkloadIntent::Job(job), &alloc)
        .expect("a job has a continuously running allocation spec");
    assert_carries_volume("the rebuilt live-allocation spec", &spec);
}
