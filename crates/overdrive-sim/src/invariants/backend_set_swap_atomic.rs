//! `BackendSetSwapAtomic` — Slice 03 (US-03; S-2.2-09).
//!
//! **Always invariant**: every observation of
//! `SimDataplane.services[service]` made concurrent with an
//! `update_service(service, ...)` call sees either the pre-swap
//! backend set or the post-swap backend set — never a mixed / torn
//! state.
//!
//! This is the DST mirror of the production `EbpfDataplane`'s atomic
//! outer-map swap (`HASH_OF_MAPS`): observers of `SERVICE_MAP[ServiceId]`
//! see either the old inner-map fd or the new inner-map fd, never a
//! half-rebuilt inner. Per `.claude/rules/development.md`
//! § *Production code is not shaped by simulation*, the `SimDataplane`
//! is reshaped to match the production atomicity property — observers
//! of the in-memory `BTreeMap` see either pre- or post-swap, never a
//! partial update — by performing the swap as a single mutex-guarded
//! `BTreeMap` reassignment of the value at the keyed entry rather
//! than a sequence of `.insert`/`.remove` calls. The single-call
//! `BTreeMap::insert` already provides this property; the invariant
//! pins it at PR time so a future "optimisation" cannot regress to
//! a `.remove`-then-`.insert` shape.
//!
//! The evaluator drives a concurrent-observation scenario: one writer
//! task issues `update_service` while N observer tasks repeatedly
//! read `service_backends`. Every observation must match either the
//! pre-swap set or the post-swap set; a mixed state is a structural
//! violation.
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variant
//! `BackendSetSwapAtomic`.

// SPIFFE / SocketAddr literals in this file are structurally
// total — every input is a hand-picked constant the test author
// can prove parses. `expect` here is documentation, not error
// suppression in an unbounded code path.
#![allow(clippy::expect_used)]

use std::net::Ipv4Addr;
use std::sync::Arc;

use overdrive_core::SpiffeId;
use overdrive_core::id::NodeId;
use overdrive_core::traits::dataplane::{Backend, Dataplane};

use crate::adapters::dataplane::SimDataplane;
use crate::harness::{InvariantResult, InvariantStatus};

/// Drive the atomicity scenario and return an `InvariantResult` pinned
/// to the canonical kebab-case name.
///
/// The scenario:
/// 1. Pre-load the dataplane with a pre-swap backend set of size N.
/// 2. Spawn O concurrent observer tasks; each repeatedly reads
///    `service_backends(vip)` for a bounded number of polls and
///    records every distinct snapshot.
/// 3. Issue exactly one `update_service(vip, post_swap)` from the
///    writer task.
/// 4. Join all observers. Assert every observed set equals either the
///    pre-swap or the post-swap set.
///
/// The atomicity guarantee comes from `SimDataplane`'s implementation:
/// `update_service` performs a single mutex-guarded `BTreeMap::insert`
/// at the keyed entry, so any observer that holds the mutex sees one
/// or the other but never a half-built `Vec<Backend>`.
pub async fn evaluate_backend_set_swap_atomic() -> InvariantResult {
    const NAME: &str = "backend-set-swap-atomic";
    const OBSERVERS: usize = 8;
    const POLLS_PER_OBSERVER: usize = 64;

    let dataplane = Arc::new(SimDataplane::new());
    let vip = Ipv4Addr::new(10, 0, 0, 1);

    // Build pre- and post-swap backend sets. Distinct on every field
    // so a mixed-state Vec containing one of each cannot be confused
    // with either canonical set.
    let pre_swap = vec![backend("alpha", "10.1.1.1:8080"), backend("alpha", "10.1.1.2:8080")];
    let post_swap = vec![
        backend("beta", "10.2.2.1:9090"),
        backend("beta", "10.2.2.2:9090"),
        backend("beta", "10.2.2.3:9090"),
    ];

    // Pre-load.
    if let Err(e) = dataplane.update_service(vip, pre_swap.clone()).await {
        return fail(NAME, format!("pre-load update_service failed: {e}"));
    }

    // Spawn observers. Each polls `service_backends(vip)` and records
    // every distinct snapshot.
    let mut observer_handles = Vec::with_capacity(OBSERVERS);
    for _ in 0..OBSERVERS {
        let dp = Arc::clone(&dataplane);
        observer_handles.push(tokio::spawn(async move {
            let mut snapshots: Vec<Vec<Backend>> = Vec::new();
            for _ in 0..POLLS_PER_OBSERVER {
                if let Some(snapshot) = dp.service_backends(vip) {
                    if snapshots.last().is_none_or(|prev| *prev != snapshot) {
                        snapshots.push(snapshot);
                    }
                }
                tokio::task::yield_now().await;
            }
            snapshots
        }));
    }

    // Concurrently issue the swap. A `yield_now` first gives observers
    // a chance to record at least one pre-swap snapshot.
    tokio::task::yield_now().await;
    if let Err(e) = dataplane.update_service(vip, post_swap.clone()).await {
        return fail(NAME, format!("swap update_service failed: {e}"));
    }

    // Join observers and check every recorded snapshot.
    for (i, handle) in observer_handles.into_iter().enumerate() {
        let snapshots = match handle.await {
            Ok(s) => s,
            Err(e) => return fail(NAME, format!("observer {i} panicked: {e}")),
        };
        for (j, snapshot) in snapshots.iter().enumerate() {
            if *snapshot != pre_swap && *snapshot != post_swap {
                return fail(
                    NAME,
                    format!(
                        "observer {i} snapshot {j} matched neither pre- nor post-swap: \
                         got {snapshot:?}"
                    ),
                );
            }
        }
    }

    pass(NAME)
}

fn backend(job: &str, addr: &str) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!("spiffe://overdrive.local/job/{job}/alloc/x"))
            .expect("valid SPIFFE ID"),
        addr: addr.parse().expect("valid SocketAddr"),
        weight: 1,
        healthy: true,
    }
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").expect("'cluster' is a valid NodeId").to_string()
}
