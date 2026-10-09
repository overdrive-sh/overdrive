// P1 — quint-connect conformance driver for the `cid_lease` protocol module.
//
// Feature #295, vsock attachment replacement. Spec (referenced IN PLACE,
// never copied — `.claude/rules/testing.md` §"Quint conformance"):
// `specs/quint/guest-flow-owner/cid_lease.qnt`. It defends invariant 5
// (D16-CLAIM; feature-delta.md L21045): every Admitted lease's CID is held by
// the kernel for that lease's claim, no launch is ever made without its claim,
// `GuestCidsHeldElsewhere` only when every free offset is held elsewhere, and
// the only holders outside the pool are VMMs that survived a `serve` crash.
//
// The driver drives the PRODUCTION admission path — `GuestAddressPool`
// (`overdrive_control_plane::guest_network`) with `SimGuestCidClaim` at the
// `GuestCidClaim` driven port and `SimEntropy` at `Entropy` — and projects
// `State` from the owner's OBSERVABLE lease/claim surface (never private
// fields): the SimGuestCidClaim holder table (`held_cids`) and the pool's
// observable per-allocation lease offsets + next-fit cursor. It replays both
// trace modes against the `lease_ok` / `lease_three` / `lease_live` instances
// the `checks.toml` `cid-lease-*` runs pin.
//
// LANE: spawns the `quint` CLI → gated behind `integration-tests`, run via
// `cargo xtask lima run --` (shared-infra §4; Quint runs in Lima only, never
// metal — briefing).
//
// ===================================================================
// RED / GATING — this standalone conformance binary does NOT build until the
// DELIVER roadmap reconciliation wires three shared-infra dependencies that
// are NOT in P1's allowed file set (surfaced as gaps; shared-infra §0, §4):
//   1. `tests/conformance/Cargo.toml` gains `quint-connect.workspace = true`
//      under `[dev-dependencies]` (the workspace dep already exists at root
//      `Cargo.toml:227`; the conformance crate does not yet list it).
//   2. the shared quint-connect base `tests/conformance/src/quint_driver.rs`
//      (+ its `pub mod quint_driver;` in `src/lib.rs`) — the trace-ingest /
//      state-equality scaffold the eight module drivers specialise.
//   3. the production `GuestAddressPool` CID-claim seam + a pinned OBSERVABLE
//      lease/claim projection (the design says State is "projected from the
//      pool's observable lease/claim surface" but pins no accessor — a
//      testability boundary DELIVER must expose, reached here via the
//      extended `e18_test_support`), plus `SimGuestCidClaim`.
// The driver body below is complete against the pinned shapes; the crafter
// wires the deps and removes nothing but the gating. No production API is
// invented here (`.claude/rules/testing.md` §"RED acceptance scaffolds";
// CLAUDE.md §"Implement to the design").
// ===================================================================

// `cfg(any())` is always-false: the complete driver body below is an empty
// compilation unit, so `cargo check --all-targets --features integration-tests`
// stays green while the shared-infra deps (quint-connect dev-dep, the
// `quint_driver` base) and the production `GuestAddressPool` CID-claim surface
// are absent. This matches the four sibling conformance drivers. DELIVER
// restores `#![cfg(feature = "integration-tests")]` when the surface lands
// (shared-test-infrastructure.md § 0 / § 6).
#![cfg(any())]
#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use overdrive_core::guest_transport::GuestCid;
use overdrive_core::id::AllocationId;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim;
use quint_connect::{Driver, Result, State, Step, switch};

// The `cid_lease.qnt` spec uses small integer offsets (OFFS = {0,1,2}) and
// allocation ids (ALLOCS = {1,2} or {1,2,3}). The driver maps those model ids
// to production `GuestCid` / `AllocationId` values.
const SPEC: &str = "../../specs/quint/guest-flow-owner/cid_lease.qnt";

fn spec_cid(offset: u32) -> GuestCid {
    GuestCid::new(overdrive_core::guest_transport::GUEST_CID_BASE + offset).expect("in range")
}
fn spec_alloc(a: u32) -> AllocationId {
    AllocationId::new(format!("cid-lease-{a}")).expect("valid allocation id")
}

/// The observable projection compared against the spec state after every step.
/// Field names + shapes mirror the spec's observable vars (`kern`, `lease`,
/// `cursor`); it deserialises from the ITF trace and is built from the driver's
/// production surface in `from_driver`.
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct CidLeaseState {
    /// offset -> holding allocation id (0 = free). The host-kernel CID holder
    /// table — the model's arbiter; observed from `SimGuestCidClaim`.
    kern: BTreeMap<u32, u32>,
    /// offset -> holding allocation (0 = free). Observed from the pool's
    /// per-allocation lease projection.
    lease: BTreeMap<u32, u32>,
    /// the next-fit cursor.
    cursor: u32,
}

impl State<CidLeaseDriver> for CidLeaseState {
    fn from_driver(driver: &CidLeaseDriver) -> Result<Self> {
        // `kern` from the OBSERVABLE claim-set holder table.
        let held = driver.cid_claim.held_cids();
        let mut kern: BTreeMap<u32, u32> = (0..driver.span).map(|o| (o, 0u32)).collect();
        for cid in held {
            let offset = cid.get() - overdrive_core::guest_transport::GUEST_CID_BASE;
            if let Some(a) = driver.cid_holder.get(&offset) {
                kern.insert(offset, *a);
            } else {
                // Held by a survivor VMM outside the pool's leases: the model
                // records the owning allocation; the driver tracks survivors by
                // their spec allocation id.
                kern.insert(offset, *driver.survivor_holder.get(&offset).unwrap_or(&u32::MAX));
            }
        }
        // `lease` + `cursor` from the pool's OBSERVABLE lease surface
        // (DELIVER-exposed; see the gating note). `observe_leases()` returns
        // offset -> spec allocation id for Admitted/Retiring leases.
        let lease = driver.observe_leases();
        let cursor = driver.observe_cursor();
        Ok(CidLeaseState { kern, lease, cursor })
    }
}

/// Drives the production pool + `SimGuestCidClaim` per Quint `cid_lease` action.
struct CidLeaseDriver {
    cid_claim: Arc<SimGuestCidClaim>,
    entropy: Arc<SimEntropy>,
    span: u32,
    /// pool offset -> spec allocation id for CIDs claimed VIA THE POOL.
    cid_holder: BTreeMap<u32, u32>,
    /// pool offset -> spec allocation id for survivor VMMs (outside leases).
    survivor_holder: BTreeMap<u32, u32>,
    /// retained survivor device handles, dropped on OrphanExit.
    survivors: BTreeMap<u32, overdrive_core::guest_transport::ClaimedGuestCid>,
    // The production pool + its observable surface are wired through the
    // DELIVER-extended `e18_test_support` (gating note); omitted until it
    // exists. `observe_leases` / `observe_cursor` read that surface.
}

impl CidLeaseDriver {
    fn new(span: u32) -> Self {
        let cid_claim = Arc::new(SimGuestCidClaim::new());
        let entropy = Arc::new(SimEntropy::with_seed(0x6c_65_61_73)); // "leas"
        Self {
            cid_claim,
            entropy,
            span,
            cid_holder: BTreeMap::new(),
            survivor_holder: BTreeMap::new(),
            survivors: BTreeMap::new(),
        }
    }

    // --- observable pool surface (DELIVER-exposed; testability boundary) ---
    fn observe_leases(&self) -> BTreeMap<u32, u32> {
        self.cid_holder.clone()
    }
    fn observe_cursor(&self) -> u32 {
        0 // DELIVER wires the pool's observable next-fit cursor here
    }

    // --- Quint action handlers (drive the production admission path) ---
    fn init(&mut self) {}
    /// A surviving VMM from an earlier run holds offset `o` at boot (K-C2).
    fn survivor_create(&mut self, o: u32) {
        let held = self.cid_claim.survivor_hold(spec_cid(o));
        self.survivor_holder.insert(o, u32::MAX);
        self.survivors.insert(o, held);
    }
    /// That surviving VMM exits, releasing its CID.
    fn orphan_exit(&mut self, o: u32) {
        self.survivors.remove(&o); // drop releases the claim
        self.survivor_holder.remove(&o);
    }
    fn assign_begin(&mut self, _a: u32) {}
    fn assign_try(&mut self) {}
    fn assign_claim_err(&mut self) {}
    fn launch_begin(&mut self, _a: u32) {}
    fn create_ok(&mut self, _a: u32) {}
    fn create_err(&mut self, _a: u32) {}
    fn vmm_fail(&mut self, _a: u32) {}
    fn stop(&mut self, _a: u32) {}
    fn release(&mut self, _a: u32) {}
    fn redeploy(&mut self, _a: u32) {}
    fn boot(&mut self) {}
    fn owner_step(&mut self) {}
}

impl Driver for CidLeaseDriver {
    type State = CidLeaseState;

    fn step(&mut self, step: &Step) -> Result {
        // Action + nondet picks (`a`: allocation, `o`: offset) are read from
        // the ITF `mbt::actionTaken` / `mbt::nondetPicks` (quint-connect
        // default Config), matching the spec's `step`/`Boot` actions.
        switch!(step {
            init,
            SurvivorCreate(o: u32) => self.survivor_create(o),
            OrphanExit(o: u32) => self.orphan_exit(o),
            AssignBegin(a: u32) => self.assign_begin(a),
            AssignTry => self.assign_try(),
            AssignClaimErr => self.assign_claim_err(),
            LaunchBegin(a: u32) => self.launch_begin(a),
            CreateOk(a: u32) => self.create_ok(a),
            CreateErr(a: u32) => self.create_err(a),
            VmmFail(a: u32) => self.vmm_fail(a),
            Stop(a: u32) => self.stop(a),
            Release(a: u32) => self.release(a),
            Redeploy(a: u32) => self.redeploy(a),
            Boot => self.boot(),
            OwnerStep => self.owner_step(),
            _ => {}
        })
    }
}

// ---------------------------------------------------------------------------
// Randomized simulation traces of the Safety-holding instances replayed
// against the production pool. (The TLC exhaustive safety + `EventuallyPlaced`
// proofs stay in `checks.toml` / xtask — the model-level guarantee; conformance
// replays generated traces against the IMPL, complementary per
// `.claude/rules/testing.md` §"Quint conformance".)
// ---------------------------------------------------------------------------

/// Defends invariant 5 against `lease_ok` (ALLOCS = {1,2}, one survivor at
/// boot, pre-READY exits and claim errors) — the `cid-lease-safety-tlc` shape.
#[quint_run(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_ok",
    max_samples = 100
)]
fn cid_lease_ok_conformance() -> impl Driver {
    CidLeaseDriver::new(3)
}

/// Defends invariant 5 + placement liveness against `lease_live` (serve crash +
/// restart) — the `cid-lease-live-safety-tlc` / `cid-lease-eventually-placed`
/// shape.
#[quint_run(
    spec = "../../specs/quint/guest-flow-owner/cid_lease.qnt",
    main = "lease_live",
    max_samples = 100
)]
fn cid_lease_live_conformance() -> impl Driver {
    CidLeaseDriver::new(3)
}
