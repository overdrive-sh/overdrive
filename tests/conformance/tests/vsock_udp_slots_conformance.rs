// P3-S8 — the production guest datagram-slot path conforms to the `udp_slots`
// Quint model along generated traces (`.claude/rules/testing.md` §"Quint
// conformance"; ADR-0168 decision QUINT). The driver drives the production
// owner + guest slot model through their driving ports (Sim at driven ports),
// projects `State` from the **observable** surface (never private fields),
// references the spec **in place** (`specs/quint/guest-flow-owner/udp_slots.qnt`,
// never copied), and runs both trace modes. Spec:
// `test-scenarios-p3-udp-slots-framing.md` P3-S8; invariant→run map there.
//
// GATED (shared-infra §0): depends on the shared conformance harness
// `boot_vsock_node` / `ConformanceNode` (shared-infra §2) and the shared
// quint-connect base `tests/conformance/src/quint_driver.rs` (shared-infra §4)
// — both first-needer-created, NOT edited by P3 (G-3). The exact `quint-connect`
// 0.1.2 driver API is not vendorable offline (G-4); this driver is authored
// against the shape shared-infra §4 + `.claude/rules/testing.md` describe (trace
// ingest, per-step dispatch trait, state-equality harness, `#[quint_test]` for
// the named `checks.toml` runs, `#[quint_run]` randomized with the seed printed
// on failure). The crafter reconciles to the real API when the shared base
// lands. This binary compiles only under `--features integration-tests`
// (conformance spawns the `quint` CLI; run via `cargo xtask lima run --`).

// `cfg(any())` is always-false: the complete driver body below is an empty
// compilation unit, so `cargo check --all-targets --features integration-tests`
// stays green while the shared conformance harness (`boot_vsock_node` /
// `ConformanceNode`) and the `quint_driver` base are absent. This matches the
// four sibling conformance drivers. DELIVER restores
// `#![cfg(feature = "integration-tests")]` when they land (shared-infra § 0/§ 6).
#![cfg(any())]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use overdrive_system_conformance::quint_driver::{QuintModel, QuintStateProjection, StepOutcome};
use overdrive_system_conformance::vsock_harness::{ConformanceNode, boot_vsock_node};
use quint_connect::{Trace, quint_run, quint_test};

/// The spec in place — never copied into this crate (one spec, one oracle).
const UDP_SLOTS_SPEC: &str = "specs/quint/guest-flow-owner/udp_slots.qnt";

/// Observable projection of the `udp_slots` model state, read from the owner's
/// datagram-association table and the guest slot pool's observable snapshot via
/// `ConformanceNode` accessors — port-exposed names only (never `slots`/`host`
/// private fields). Mirrors the model's `Slot` / `Assoc` / control-message sets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct UdpSlotsState {
    /// slot index -> (owned, sock, dest, fid, routed, parked tags, holders)
    slots: BTreeMap<u32, ObservedSlot>,
    /// host flow id -> association state ("none" | "req" | "paired" | "closed")
    host: BTreeMap<u32, String>,
    to_guest_paired: BTreeSet<u32>,
    to_guest_refused: BTreeSet<u32>,
    to_guest_abort: BTreeSet<u32>,
    to_host_abort: BTreeSet<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSlot {
    owned: bool,
    sock: i64,
    dest: i64,
    fid: i64,
    routed: bool,
    parked: BTreeSet<(i64, i64)>,
    holders: i64,
}

/// The conformance model: maps each `udp_slots` action to a call on the
/// production datagram-slot path and projects the observable state after it.
struct UdpSlotsConformance {
    node: ConformanceNode,
}

impl QuintModel for UdpSlotsConformance {
    type State = UdpSlotsState;

    /// Map one Quint action name + args to a production call. Returns
    /// `StepOutcome::Applied` when the production path accepts the action, or
    /// `StepOutcome::Skipped` when the model action has no production counterpart
    /// at the current state (the harness asserts the guard matched the model).
    fn step(&mut self, action: &str, args: &Trace) -> StepOutcome {
        match action {
            // application sends / slot claim
            "AppClaim" => {
                self.node.guest_first_datagram(args.int("k"), args.int("d"), args.int("s"))
            }
            "AppSendOnSlot" => {
                self.node.guest_datagram_on_slot(args.int("k"), args.int("d"), args.int("s"))
            }
            // readiness dispatch by (slot, current-generation?)
            "GuestReady" => self.node.guest_dispatch_readiness(args.int("s"), args.bool("cur")),
            // host pair / refuse / abort
            "HostPair" => self.node.owner_pair(args.int("i")),
            "HostRefuse" => self.node.owner_refuse(args.int("i")),
            "HostAbortAssoc" => self.node.owner_abort(args.int("i")),
            // guest receives Paired / Refused / Abort; pairing deadline
            "GuestRecvPaired" => self.node.guest_recv_paired(args.int("i")),
            "GuestRecvEnd" => self.node.guest_recv_end(args.int("i")),
            "GuestPairingTimeout" => self.node.guest_pairing_timeout(args.int("s")),
            "HostRecvEnd" => self.node.owner_recv_end(args.int("i")),
            // slot release paths
            "SockClose" => self.node.guest_close_socket(args.int("k")),
            "SockCloseReportLost" => self.node.guest_close_socket_report_lost(args.int("k")),
            "GuestReleaseSock" => self.node.guest_release_socket(args.int("k")),
            "GuestAudit" => self.node.guest_audit_orphans(),
            "IdleRelease" => self.node.guest_idle_release(args.int("s")),
            other => StepOutcome::Unmapped(other.to_string()),
        }
    }

    fn project(&self) -> QuintStateProjection<Self::State> {
        // Read only the observable surface of the node.
        QuintStateProjection::new(UdpSlotsState {
            slots: self.node.observe_slots(),
            host: self.node.observe_host_associations(),
            to_guest_paired: self.node.observe_control_to_guest_paired(),
            to_guest_refused: self.node.observe_control_to_guest_refused(),
            to_guest_abort: self.node.observe_control_to_guest_abort(),
            to_host_abort: self.node.observe_control_to_host_abort(),
        })
    }
}

async fn fresh_model() -> UdpSlotsConformance {
    // The production composition with the vsock transport, Sim at the
    // external/non-deterministic driven ports (clock, entropy, VMM), driving the
    // guest datagram-slot path.
    UdpSlotsConformance { node: boot_vsock_node(Default::default()).await }
}

// ---------------------------------------------------------- named CI runs
// Each #[quint_test] replays the trace of a named `checks.toml` run against the
// production path and compares the projected state after every step. The hazard
// runs are the model's own teeth (they prove each invariant bites) and are NOT
// replayed against code — the conformance set is the design-instance checks.

/// Invariants 2 (SlotHeldOnce, ReleasedSlotCarriesNoData, RouteCurrent,
/// LatePairedAborted/SLOT-ABORT, NoReassociationWithoutNewDatagram) + 14
/// (NoStaleGenerationStart) hold along the exhaustive TLC safety trace.
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-safety-tlc")]
#[ignore = "pending DELIVER: udp_slots conformance driver + shared quint_driver/boot_vsock_node base"]
async fn conforms_to_udp_slots_safety_tlc() {
    let mut model = fresh_model().await;
    quint_connect::replay_named(&mut model, UDP_SLOTS_SPEC, "udp-slots-safety-tlc")
        .await
        .assert_state_matches_every_step();
}

/// The Apalache bounded safety trace (2 destinations, 3 association ids).
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-safety-apalache")]
#[ignore = "pending DELIVER: udp_slots conformance driver + shared base"]
async fn conforms_to_udp_slots_safety_apalache() {
    let mut model = fresh_model().await;
    quint_connect::replay_named(&mut model, UDP_SLOTS_SPEC, "udp-slots-safety-apalache")
        .await
        .assert_state_matches_every_step();
}

/// Progress: a slot of a released socket returns to the pool.
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-release-liveness")]
#[ignore = "pending DELIVER: udp_slots conformance driver + shared base"]
async fn conforms_to_released_slot_becomes_free() {
    let mut model = fresh_model().await;
    quint_connect::replay_named(&mut model, UDP_SLOTS_SPEC, "udp-slots-release-liveness")
        .await
        .assert_state_matches_every_step();
}

/// The non-vacuity witnesses: each reaches a reachable state (late Paired
/// answered, a routed slot, two slots for one socket, a reassociation, a stale
/// cell-generation event) so the invariants are not vacuously true.
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-witness-late-paired")]
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-witness-routed")]
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-witness-two-slots-one-socket")]
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-witness-reassociation")]
#[quint_test(spec = UDP_SLOTS_SPEC, run = "udp-slots-witness-stale-event")]
#[ignore = "pending DELIVER: udp_slots conformance driver + shared base"]
async fn reaches_each_udp_slots_witness(run: &str) {
    let mut model = fresh_model().await;
    quint_connect::replay_named(&mut model, UDP_SLOTS_SPEC, run)
        .await
        .assert_state_matches_every_step();
}

// ---------------------------------------------------------- randomized traces
/// Randomized simulation traces of the design instance `slots_ok`; the seed is
/// printed on failure (same reproducibility rules as DST).
#[quint_run(spec = UDP_SLOTS_SPEC, main = "slots_ok", steps = 200)]
#[ignore = "pending DELIVER: udp_slots conformance driver + shared base"]
async fn conforms_under_randomized_udp_slots_traces(trace: Trace) {
    let mut model = fresh_model().await;
    quint_connect::replay_trace(&mut model, trace).await.assert_state_matches_every_step();
}
