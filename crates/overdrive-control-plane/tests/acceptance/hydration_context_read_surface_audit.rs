//! S-ROH-B-09 — `HydrationContext` S1 read-surface audit (ADR-0086 D5 / S1).
//!
//! **CONTRACT_SHAPE: structural (S1 acceptance gate).** This is the PRIMARY
//! ADR-0086 S1 evidence. The gate is the "read every `hydrate_*` body" audit:
//! `HydrationContext` MUST carry a handle for EVERY surface any moved
//! `hydrate_*` body reads, and NO more — post-ADR-0087 there is NO
//! restart-budget surface.
//!
//! At step 02-01 the per-reconciler `hydrate_*` bodies are still
//! `todo!("RED scaffold")` (they move onto the impls in 02-04). So this audit
//! is authored as the **read-surface enumeration over the CENTRAL free fns
//! that WILL move** — the exact surfaces
//! `overdrive-control-plane/src/reconciler_runtime.rs::{hydrate_desired,
//! hydrate_actual}` (and their nine `hydrate_*_*` helpers) read today:
//!
//! | Surface the central hydrate bodies read | `HydrationContext` field |
//! |---|---|
//! | `state.store` (`&dyn IntentStore` — `get` / `scan_prefix`) | `intent_store` |
//! | `state.obs` (`&dyn ObservationStore` — `*_rows`) | `observation_store` |
//! | `state.drivers.get(Vm).live_allocations()` (`DriverRegistry`) | `drivers` |
//! | `state.vm_host_state.observe()` (`&dyn VmHostState`) | `vm_host_state` |
//! | `state.listener_facts.lock().await.fact_for(..)` | `listener_facts` |
//! | `state.allocator.lock().await.get(..)` (VIP memo) | `service_vip_view` |
//! | `state.engine.live_instances()` | `workflow_live_set` |
//! | `state.identity.held_snapshot()` | `held_svid_view` |
//! | plain `node_id` | `node_id` |
//! | plain `host_ipv4` | `host_ipv4` |
//! | plain `intent_redb_path` | `intent_redb_path` |
//!
//! Two structural audit functions below ARE the assertion. They never run;
//! their *compilation* is the gate:
//!
//! * `audit_every_read_surface_is_represented` — proves each surface is a
//!   `HydrationContext` field of the expected type (the "every surface
//!   represented" clause). A missing / wrongly-typed field fails to compile.
//! * `audit_field_set_is_exactly_the_read_surfaces` — an EXHAUSTIVE destructure
//!   (no `..`) proves the field set is EXACTLY these surfaces. If a
//!   restart-budget field were reintroduced (ADR-0087 removed the cross-read),
//!   or any other unrepresented `state.*` field added, the pattern fails to
//!   compile ("pattern does not mention field ..."); if a surface is dropped,
//!   the pattern names a nonexistent field. This is the "no unrepresented
//!   `state.*` and no restart-budget surface" clause, made structural.
//!
//! This test needs NO live port instances (those arrive with the 02-05 Sim
//! read-ports), so it is fully live at 02-01. It becomes doubly-binding once
//! the hydrate bodies move in 02-04: any body that reaches a `state.*` field
//! not on `HydrationContext` cannot be written against `ctx`.

// The two audit fns are a structural type-check shape, not runtime logic: they
// take a value by design (to destructure the exact field set), bind
// `let _foo: Type = ...` purely to assert the field's type, and are non-const.
// These lints are inherent to that shape, so they are allowed file-wide.
#![allow(
    clippy::doc_markdown,
    clippy::no_effect_underscore_binding,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value
)]

use std::net::Ipv4Addr;
use std::path::Path;

use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::HydrationContext;
use overdrive_core::traits::driver::DriverRegistry;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::{HeldSvidView, ListenerFacts, ServiceVipView, WorkflowLiveSet};

/// Audit 1 — TYPE coverage. Compiles only if `HydrationContext` carries every
/// read surface the central `hydrate_*` free fns touch, each at the expected
/// type. Never called; its type-check IS the assertion.
fn audit_every_read_surface_is_represented(ctx: &HydrationContext<'_>) {
    let _intent: &dyn IntentStore = ctx.intent_store;
    let _obs: &dyn ObservationStore = ctx.observation_store;
    let _drivers: &DriverRegistry = ctx.drivers;
    let _vm_host: &dyn VmHostState = ctx.vm_host_state;
    let _listener: &dyn ListenerFacts = ctx.listener_facts;
    let _vip: &dyn ServiceVipView = ctx.service_vip_view;
    let _workflow: &dyn WorkflowLiveSet = ctx.workflow_live_set;
    let _held: &dyn HeldSvidView = ctx.held_svid_view;
    let _node: &NodeId = ctx.node_id;
    let _host_ipv4: Ipv4Addr = ctx.host_ipv4;
    let _redb: &Path = ctx.intent_redb_path;
}

/// Audit 2 — EXACT field set. An exhaustive destructure (no `..`) fails to
/// compile if `HydrationContext` grows a field (e.g. a restart-budget surface
/// — ADR-0087 removed that cross-read) OR drops one. This is the machine-checked
/// "no unrepresented `state.*`, no restart-budget surface" clause. Never called.
fn audit_field_set_is_exactly_the_read_surfaces(ctx: HydrationContext<'_>) {
    let HydrationContext {
        intent_store: _,
        observation_store: _,
        drivers: _,
        vm_host_state: _,
        listener_facts: _,
        service_vip_view: _,
        workflow_live_set: _,
        held_svid_view: _,
        node_id: _,
        host_ipv4: _,
        intent_redb_path: _,
    } = ctx;
}

/// S-ROH-B-09 — the S1 read-surface audit gate.
///
/// The two `audit_*` functions above are the load-bearing structural
/// assertions (they compile only when the `HydrationContext` field set is
/// exactly the ADR-0086 D5 read surfaces). This `#[test]` references them so
/// they are type-checked and monomorphised, then documents the audited surface
/// list for the human reader and pins that no restart-budget surface is present.
#[test]
fn hydration_context_covers_every_hydrate_read_surface_and_carries_no_restart_budget() {
    // Force both compile-time audits to type-check — they ARE the gate.
    let _type_audit: fn(&HydrationContext<'_>) = audit_every_read_surface_is_represented;
    let _exact_audit: fn(HydrationContext<'_>) = audit_field_set_is_exactly_the_read_surfaces;

    // The read surfaces the moved `hydrate_*` bodies consume (ADR-0086 D5),
    // enumerated for the human reader; the exhaustive destructure above is the
    // machine-checked twin. `restart_budget` is deliberately ABSENT —
    // ADR-0087 (single restart authority) removed the cross-reconciler
    // restart-budget read at its root, so no `RestartBudgetView` port exists.
    let audited_surfaces: [&str; 11] = [
        "intent_store",
        "observation_store",
        "drivers",
        "vm_host_state",
        "listener_facts",
        "service_vip_view",
        "workflow_live_set",
        "held_svid_view",
        "node_id",
        "host_ipv4",
        "intent_redb_path",
    ];

    assert!(
        !audited_surfaces.contains(&"restart_budget"),
        "ADR-0087 removed the restart-budget cross-read; no HydrationContext \
         surface may reintroduce it"
    );
    assert_eq!(
        audited_surfaces.len(),
        11,
        "ADR-0086 D5 pins exactly 11 read surfaces (8 injected handles + \
         node_id + host_ipv4 + intent_redb_path)"
    );
}
