//! Shared fixtures and proptest strategies for `overdrive-scheduler`
//! acceptance tests.
//!
//! Mirrors `crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs`
//! generator shape — DNS-1123-style labels for ID newtypes, bounded
//! resource ranges that exercise both fits-everywhere and exhausted
//! cases.
//!
//! # Phase-1 capacity model
//!
//! `AllocStatusRow` carries no per-alloc `Resources` field today (see
//! `overdrive-core::traits::observation_store::AllocStatusRow`; pinned
//! REUSE AS-IS by `docs/feature/phase-1-first-workload/design/wave-
//! decisions.md`). The scheduler therefore treats each `Running`
//! allocation targeting a node as reserving the resource envelope of
//! the *new* job being placed — adequate for Phase 1's homogeneous-
//! workload first-fit semantics. Phase 2+ will add a `resources` field
//! to `AllocStatusRow` and switch the scheduler to per-alloc accounting.

#![allow(dead_code)] // helpers are referenced from sibling modules; nextest
// sees each module file independently.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use proptest::prelude::*;

use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

// ---------------------------------------------------------------------------
// Hand-built fixtures (used by point-tests that don't need proptest)
// ---------------------------------------------------------------------------

#[must_use]
pub fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

#[must_use]
pub fn jid(s: &str) -> JobId {
    JobId::new(s).expect("valid JobId")
}

#[must_use]
pub fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

#[must_use]
pub fn region(s: &str) -> Region {
    Region::new(s).expect("valid Region")
}

#[must_use]
pub const fn res(cpu_milli: u32, memory_bytes: u64) -> Resources {
    Resources { cpu_milli, memory_bytes }
}

#[must_use]
pub fn make_node(id: &str, capacity: Resources) -> Node {
    Node { id: nid(id), region: region("local"), capacity }
}

#[must_use]
pub fn make_job(id: &str, resources: Resources) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources,
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}

#[must_use]
pub fn make_alloc_running(alloc_id: &str, job_id: &str, target_node: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(target_node),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(target_node) },
        reason: None,
        detail: None,
    }
}

#[must_use]
pub fn make_alloc_terminated(alloc_id: &str, job_id: &str, target_node: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(target_node),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(target_node) },
        reason: None,
        detail: None,
    }
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

const ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const ALNUM_DASH: &str = "abcdefghijklmnopqrstuvwxyz0123456789-";
const ALNUM: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// Valid DNS-1123 label matching `JobId` / `NodeId` / `Region` shape.
/// Same generator overdrive-core uses in `aggregate_roundtrip.rs`.
pub fn valid_label() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()).prop_map(|c| c.to_string()),
        (
            proptest::sample::select(ALPHA.chars().collect::<Vec<_>>()),
            prop::collection::vec(
                proptest::sample::select(ALNUM_DASH.chars().collect::<Vec<_>>()),
                0..=10,
            ),
            proptest::sample::select(ALNUM.chars().collect::<Vec<_>>()),
        )
            .prop_map(|(first, interior, last)| {
                let mut s = String::with_capacity(2 + interior.len());
                s.push(first);
                s.extend(interior);
                s.push(last);
                s
            }),
    ]
}

/// An arbitrary valid `Resources` envelope. Bounded so the proptest
/// exercises both "fits everywhere" and "exhausted" cases.
pub fn arb_resources() -> impl Strategy<Value = Resources> {
    (0u32..=64_000u32, 1u64..=(64u64 * 1024 * 1024 * 1024))
        .prop_map(|(cpu_milli, memory_bytes)| Resources { cpu_milli, memory_bytes })
}

/// An arbitrary valid `Node` with the given id label.
fn arb_node_with_id(id: String) -> impl Strategy<Value = Node> {
    arb_resources().prop_map(move |capacity| Node {
        id: NodeId::new(&id).expect("valid NodeId"),
        region: Region::new("local").expect("'local' is a valid Region"),
        capacity,
    })
}

/// A small `BTreeMap<NodeId, Node>` (1..=4 entries) suitable for
/// per-scenario placement testing.
pub fn arb_node_map() -> BoxedStrategy<BTreeMap<NodeId, Node>> {
    prop::collection::vec(valid_label(), 1..=4)
        .prop_flat_map(|labels| {
            let dedup: Vec<String> = {
                let mut seen = std::collections::BTreeSet::new();
                labels.into_iter().filter(|l| seen.insert(l.clone())).collect()
            };
            let strategies: Vec<_> = dedup.into_iter().map(arb_node_with_id).collect();
            strategies.prop_map(|nodes| {
                let mut map = BTreeMap::new();
                for n in nodes {
                    map.insert(n.id.clone(), n);
                }
                map
            })
        })
        .boxed()
}

/// An arbitrary valid `Job`.
pub fn arb_job() -> impl Strategy<Value = Job> {
    (valid_label(), 1u32..=64u32, arb_resources()).prop_map(|(id, replicas, resources)| Job {
        id: JobId::new(&id).expect("valid JobId"),
        replicas: NonZeroU32::new(replicas).expect("replicas > 0"),
        resources,
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    })
}

/// A small `Vec<AllocStatusRow>` (0..=4 entries) referencing the given
/// node ID set. Each alloc targets one of the input nodes uniformly at
/// random — this matches the §18 observation row shape: only allocations
/// pinned to a node by the placement reconciler appear here.
pub fn arb_allocs_for_nodes(node_ids: Vec<NodeId>) -> BoxedStrategy<Vec<AllocStatusRow>> {
    if node_ids.is_empty() {
        return Just(Vec::<AllocStatusRow>::new()).boxed();
    }
    let max_idx = node_ids.len() - 1;
    (prop::collection::vec((0usize..=max_idx, valid_label(), valid_label(), any::<bool>()), 0..=4),)
        .prop_map(move |(rows,)| {
            rows.into_iter()
                .map(|(idx, alloc_label, job_label, is_running)| {
                    let target = node_ids[idx].clone();
                    AllocStatusRow {
                        alloc_id: AllocationId::new(&alloc_label).expect("valid AllocationId"),
                        job_id: JobId::new(&job_label).expect("valid JobId"),
                        node_id: target.clone(),
                        reason: None,
                        detail: None,
                        state: if is_running {
                            AllocState::Running
                        } else {
                            AllocState::Terminated
                        },
                        updated_at: LogicalTimestamp { counter: 1, writer: target },
                    }
                })
                .collect()
        })
        .boxed()
}
