//! Acceptance scenarios for phase-1-control-plane-core step 01-01 —
//! `Job` / `Node` / `Allocation` validating constructors.
//!
//! Covers the §2.1 happy-path scenarios from
//! `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`:
//!
//! * Scenario 4 — constructing a `Node` reuses the `Resources` type already
//!   exposed by the driver trait.
//! * Scenario 5 — an `Allocation` links a `Job` and a `Node` through typed
//!   newtypes only; no raw `String` / `u64` appear in the public field
//!   signatures.
//!
//! Also asserts that `Resources` is declared exactly once in the
//! `overdrive-core` source tree (no duplicate type emerges under a new
//! name inside `aggregate/`).
//!
//! Scenario 1 (rkyv round-trip) is owned by step 01-03 and is deliberately
//! NOT covered here.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::any::{TypeId, type_name};
use std::num::NonZeroU32;

use overdrive_core::aggregate::{
    Allocation, AllocationSpecInput, DriverInput, ExecInput, Job, JobSpecInput, Node,
    NodeSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::driver::Resources;

// ---------------------------------------------------------------------------
// Happy-path constructor scenarios (AC bullet 1 + 2)
// ---------------------------------------------------------------------------

#[test]
fn job_from_spec_accepts_canonical_input() {
    // Given the canonical happy-path input from the step's AC bullet 1.
    let spec = JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 2000, memory_bytes: 4 * 1024 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };

    // When Ana calls the validating constructor.
    let job = Job::from_spec(spec).expect("canonical input must construct");

    // Then the resulting Job carries the expected typed fields.
    assert_eq!(
        job.id,
        JobId::new("payments").expect("static valid JobId"),
        "id must be the typed JobId"
    );
    assert_eq!(
        job.replicas,
        NonZeroU32::new(1).expect("static valid replicas"),
        "replicas must round-trip through NonZeroU32"
    );
    assert_eq!(job.resources.cpu_milli, 2000, "cpu_milli must round-trip");
    assert_eq!(job.resources.memory_bytes, 4 * 1024 * 1024 * 1024, "memory_bytes must round-trip");
}

#[test]
fn node_new_accepts_canonical_input() {
    // Given a canonical Node spec.
    let spec = NodeSpecInput {
        id: "worker-01".to_string(),
        region: "eu-west-1".to_string(),
        cpu_milli: 8000,
        memory_bytes: 16 * 1024 * 1024 * 1024,
    };

    // When Ana calls the validating constructor.
    let node = Node::new(spec).expect("canonical input must construct");

    // Then the Node carries typed newtypes and a `Resources` envelope.
    assert_eq!(node.id, NodeId::new("worker-01").expect("static valid NodeId"));
    assert_eq!(node.region, Region::new("eu-west-1").expect("static valid Region"));
    assert_eq!(node.capacity.cpu_milli, 8000);
    assert_eq!(node.capacity.memory_bytes, 16 * 1024 * 1024 * 1024);
}

#[test]
fn allocation_new_accepts_canonical_input() {
    // Given a canonical Allocation spec.
    let spec = AllocationSpecInput {
        id: "a1b2c3d4".to_string(),
        job_id: "payments".to_string(),
        node_id: "worker-01".to_string(),
    };

    // When Ana calls the validating constructor.
    let alloc = Allocation::new(spec).expect("canonical input must construct");

    // Then the Allocation links the three newtypes.
    assert_eq!(alloc.id, AllocationId::new("a1b2c3d4").expect("static valid AllocationId"));
    assert_eq!(alloc.job_id, JobId::new("payments").expect("static valid JobId"));
    assert_eq!(alloc.node_id, NodeId::new("worker-01").expect("static valid NodeId"));
}

// ---------------------------------------------------------------------------
// Scenario 4 — Resources type is reused, not duplicated (AC bullet 4)
// ---------------------------------------------------------------------------

/// Compile-time witness that a value has the authoritative `Resources`
/// type from `overdrive_core::traits::driver`. Declared at module scope
/// to satisfy `clippy::items_after_statements`.
const fn assert_is_driver_resources(_: &Resources) {}

/// Count exact `pub struct Resources` declarations — matches the
/// canonical type only, NOT `pub struct ResourcesInput` (the wire-
/// shape twin per ADR-0031 §2). Three acceptable forms:
///   `pub struct Resources {`
///   `pub struct Resources<` (generic) — none today, future-proof
///   `pub struct Resources;` (unit) — none today, future-proof
fn count_resources_decls(body: &str) -> usize {
    body.matches("pub struct Resources {").count()
        + body.matches("pub struct Resources<").count()
        + body.matches("pub struct Resources;").count()
}

#[test]
fn job_resources_and_node_capacity_resolve_to_the_same_resources_type() {
    // Given a Job and a Node constructed through the public constructors.
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 1000, memory_bytes: 1_073_741_824 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("canonical Job input");
    let node = Node::new(NodeSpecInput {
        id: "worker-01".to_string(),
        region: "eu-west-1".to_string(),
        cpu_milli: 1000,
        memory_bytes: 1_073_741_824,
    })
    .expect("canonical Node input");

    // When Ana inspects the TypeId of `job.resources` and `node.capacity`.
    let job_resources_type = TypeId::of::<Resources>();
    let node_capacity_type = TypeId::of::<Resources>();

    // The compile-time type must resolve to the authoritative
    // `overdrive_core::traits::driver::Resources`.
    assert_is_driver_resources(&job.resources);
    assert_is_driver_resources(&node.capacity);

    // And the two have the same TypeId.
    assert_eq!(
        job_resources_type, node_capacity_type,
        "Job.resources and Node.capacity must resolve to the same Resources type"
    );

    // And that type name is rooted in `overdrive_core::traits::driver`, not
    // `overdrive_core::aggregate`.
    let name = type_name::<Resources>();
    assert!(
        name.contains("overdrive_core::traits::driver::Resources"),
        "Resources must come from traits::driver; got {name:?}"
    );
    assert!(
        !name.contains("aggregate"),
        "Resources must NOT be re-declared in aggregate; got {name:?}"
    );
}

#[test]
fn no_second_resources_type_exists_in_overdrive_core_sources() {
    // Grep gate — every .rs file under `crates/overdrive-core/src` may mention
    // `Resources`, but `pub struct Resources` must appear exactly once. This
    // defends against a future refactor that silently re-declares the type
    // inside `aggregate/`.
    let traits_driver = include_str!("../../src/traits/driver.rs");
    let aggregate_mod = include_str!("../../src/aggregate/mod.rs");
    let lib_rs = include_str!("../../src/lib.rs");
    let id_rs = include_str!("../../src/id.rs");
    let error_rs = include_str!("../../src/error.rs");
    let reconciler_rs = include_str!("../../src/reconciler.rs");

    let authoritative_decls = count_resources_decls(traits_driver);
    assert_eq!(
        authoritative_decls, 1,
        "authoritative Resources must be declared exactly once in traits/driver.rs"
    );

    for (label, body) in [
        ("aggregate/mod.rs", aggregate_mod),
        ("lib.rs", lib_rs),
        ("id.rs", id_rs),
        ("error.rs", error_rs),
        ("reconciler.rs", reconciler_rs),
    ] {
        assert_eq!(
            count_resources_decls(body),
            0,
            "no second Resources struct may exist in {label}"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 5 — Allocation public fields are typed newtypes only (AC bullet 3)
// ---------------------------------------------------------------------------

#[test]
fn allocation_public_fields_are_typed_newtypes_not_raw_primitives() {
    // Given an Allocation constructed through `new`.
    let alloc = Allocation::new(AllocationSpecInput {
        id: "a1b2c3d4".to_string(),
        job_id: "payments".to_string(),
        node_id: "worker-01".to_string(),
    })
    .expect("canonical input");

    // When Ana inspects each field's compile-time type name.
    let id_type = type_name_of_val(&alloc.id);
    let job_id_type = type_name_of_val(&alloc.job_id);
    let node_id_type = type_name_of_val(&alloc.node_id);

    // Then each field is a typed newtype from `overdrive_core::id`,
    // never a bare primitive.
    assert_eq!(
        id_type, "overdrive_core::id::AllocationId",
        "Allocation.id must be AllocationId; got {id_type}"
    );
    assert_eq!(
        job_id_type, "overdrive_core::id::JobId",
        "Allocation.job_id must be JobId; got {job_id_type}"
    );
    assert_eq!(
        node_id_type, "overdrive_core::id::NodeId",
        "Allocation.node_id must be NodeId; got {node_id_type}"
    );

    // And explicitly not a bare String or u64.
    for (field, ty) in [("id", id_type), ("job_id", job_id_type), ("node_id", node_id_type)] {
        assert_ne!(ty, "alloc::string::String", "{field} must not be bare String");
        assert_ne!(ty, "u64", "{field} must not be bare u64");
        assert_ne!(ty, "u32", "{field} must not be bare u32");
        assert_ne!(ty, "i64", "{field} must not be bare i64");
    }
}

#[test]
fn job_public_fields_are_typed_newtypes_not_raw_primitives() {
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 1000, memory_bytes: 1_073_741_824 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("canonical input");

    let id_type = type_name_of_val(&job.id);
    let replicas_type = type_name_of_val(&job.replicas);
    let resources_type = type_name_of_val(&job.resources);

    assert_eq!(id_type, "overdrive_core::id::JobId");
    // `type_name` renders `NonZeroU32` as the generic base — accept either
    // the historical spelling or the generic form.
    assert!(
        replicas_type == "core::num::nonzero::NonZeroU32"
            || replicas_type == "core::num::nonzero::NonZero<u32>",
        "replicas must be NonZeroU32; got {replicas_type}"
    );
    assert_eq!(resources_type, "overdrive_core::traits::driver::Resources");

    // And no bare String / u32 on the fields.
    assert_ne!(id_type, "alloc::string::String");
    assert_ne!(replicas_type, "u32", "replicas must NOT be bare u32");
}

#[test]
fn node_public_fields_are_typed_newtypes_not_raw_primitives() {
    let node = Node::new(NodeSpecInput {
        id: "worker-01".to_string(),
        region: "eu-west-1".to_string(),
        cpu_milli: 1000,
        memory_bytes: 1_073_741_824,
    })
    .expect("canonical input");

    let id_type = type_name_of_val(&node.id);
    let region_type = type_name_of_val(&node.region);
    let capacity_type = type_name_of_val(&node.capacity);

    assert_eq!(id_type, "overdrive_core::id::NodeId");
    assert_eq!(region_type, "overdrive_core::id::Region");
    assert_eq!(capacity_type, "overdrive_core::traits::driver::Resources");
}

// ---------------------------------------------------------------------------
// Helper — stable `type_name_of_val` (std has it gated behind unstable on
// older toolchains, so we roll our own).
// ---------------------------------------------------------------------------

fn type_name_of_val<T: ?Sized>(_: &T) -> &'static str {
    type_name::<T>()
}
