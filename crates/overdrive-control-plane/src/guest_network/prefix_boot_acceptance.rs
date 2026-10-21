//! P4-S14 — `probe_startup` (boot step 1, phase 1 of the owner's two-phase
//! boot, D-8) before any boot effect (seeded sim, source-local): the
//! guest-prefix configuration checks (D8-PREFIX-CFG, D8-PREFIX-VIP,
//! ADR-0185, R5-40), owner cgroup placement (D24), the fd budget, and then
//! the forwarder probe on a context phase 1 opens and releases itself
//! (`GuestFlowIo::probe_forwarder`; G-V1, ADR-0205, ADR-0159; a failure
//! refuses with `GuestNetworkError::Forwarder`). Phase 1 never opens the
//! serving context (boot step 9's `open_forwarder`) and leaves no context
//! open.
//!
//! netns-density-295 (`docs/feature/netns-density-295/feature-delta.md`
//! § "Composition — production entry points" boot step 1; § "Owner and
//! provisioner" `probe_startup`; § "Typed error taxonomy"). The operator
//! projection of these refusals through `run_server`
//! (`health.startup.refused`, reason `guest_network.probe`) is P4-S13's, in
//! `tests/integration/shared_guest_network_startup.rs`.
//!
//! Source-local because the owner's effect seam (`GuestFlowIo`) and its sim
//! are crate-private (QPROJ-1). Every test drives the production owner
//! `HostSharedGuestNetworkOwner::with_flow_io(ports, SimGuestFlowIo)` through
//! its driving port `probe_startup` and asserts only port-observable state:
//! the returned `GuestNetworkError` and the ordered effects the owner issued
//! through the seam.
//!
//! Driving ports: `probe_guest_flow_startup(io, vip_range, node_prefix)`,
//! the crate-private phase-1 function `run_server` calls before the owner
//! exists, and `HostSharedGuestNetworkOwner::probe_startup`, the same
//! function over the owner's own seam (the supplied-owner path); the
//! forwarder cell drives both and requires the same outcome.
//!
//! State-delta universe: `{probe outcome, effects issued, the forwarder
//! device's context table}`. A refusal of the configuration checks issues no
//! effect beyond the host-fact reads — not even the forwarder probe; a pass
//! issues nothing beyond those reads and the forwarder probe
//! (`ProbeForwarder`); no serving context, listener, gateway route or other
//! boot effect, and every context phase 1 opened is released (contract shape
//! unbounded-preservation).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "source-local acceptance bodies fail at once on a broken precondition"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, VsockListenPort};
use overdrive_core::net::InterfaceIndex;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::SimMtlsResolve;
use overdrive_sim::adapters::clock::SimClock;
use proptest::prelude::*;

use super::flow_io::GuestFlowIo;
use overdrive_core::guest_transport::forwarder::{
    ForwarderError, ForwarderProbeObservation, ForwarderProbeStage,
};

use super::flow_io::sim::{
    SimEffect, SimEffectKind, SimFailure, SimForwarderFailure, SimGuestFlowIo,
};
use super::{
    GuestFlowOwnerPorts, GuestNetworkError, HostSharedGuestNetworkOwner, SharedGuestNetworkOwner,
    probe_guest_flow_startup,
};

// ---------------------------------------------------------------------------
// Fixture: the production owner over the seam's sim
// ---------------------------------------------------------------------------

/// The owner's ports with the configured service VIP range `vip`; everything
/// else as in production (`GUEST_NODE_PREFIX` / `GUEST_NODE_GATEWAY`), the
/// time and resolution ports simulated.
fn owner_ports(vip: &[Ipv4Net]) -> GuestFlowOwnerPorts {
    GuestFlowOwnerPorts {
        clock: Arc::new(SimClock::new()),
        resolve: Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
        mtls: crate::shared_network_test_ports::mtls_worker(),
        vip_range: VipRange::new(vip.to_vec(), BTreeSet::new()).expect("valid VIP range"),
        node_prefix: GUEST_NODE_PREFIX,
        gateway: GUEST_NODE_GATEWAY,
        stream_port: VsockListenPort::KernelAssigned,
        control_port: VsockListenPort::KernelAssigned,
    }
}

/// A VIP range disjoint from the guest prefix (the production default shape).
fn disjoint_vip() -> Vec<Ipv4Net> {
    vec![net("10.96.0.0/16")]
}

fn net(cidr: &str) -> Ipv4Net {
    cidr.parse().expect("valid CIDR")
}

fn ifindex(raw: u32) -> InterfaceIndex {
    InterfaceIndex::new(raw).expect("a non-zero interface index")
}

/// One `serve` process: the production owner over a fresh seam.
struct Serve {
    io: Arc<SimGuestFlowIo>,
    owner: Arc<HostSharedGuestNetworkOwner>,
}

impl Serve {
    fn start(ports: GuestFlowOwnerPorts) -> Self {
        let io = SimGuestFlowIo::new(0x0295_0013);
        let seam: Arc<dyn GuestFlowIo> = Arc::clone(&io) as Arc<dyn GuestFlowIo>;
        let owner = Arc::new(HostSharedGuestNetworkOwner::with_flow_io(ports, seam));
        Self { io, owner }
    }

    /// The node's root-namespace interfaces and their IPv4 addresses.
    fn with_interfaces(self, interfaces: Vec<(InterfaceIndex, Vec<&str>)>) -> Self {
        self.io.update_host_facts(|facts| {
            facts.interfaces = interfaces
                .into_iter()
                .map(|(index, addresses)| (index, addresses.iter().map(|cidr| net(cidr)).collect()))
                .collect();
        });
        self
    }

    /// `serve` ends: the process's objects go.
    fn exit(self) {
        drop(self.owner);
        self.io.exit_process();
    }
}

/// The issued effects other than the host-fact reads — what a refusal of the
/// configuration checks must leave empty (they run before any boot effect).
fn boot_effects(io: &SimGuestFlowIo) -> Vec<SimEffect> {
    io.issued()
        .into_iter()
        .filter(|effect| {
            !matches!(effect.kind, SimEffectKind::HostFacts | SimEffectKind::StaleObjects)
        })
        .collect()
}

/// The issued effects beyond the host-fact reads and phase 1's forwarder
/// probe (D-8) — what a passing `probe_startup` leaves empty. The serving
/// context's open (`OpenForwarder`, boot step 9) is not among them.
fn effects_beyond_probe(io: &SimGuestFlowIo) -> Vec<SimEffect> {
    boot_effects(io)
        .into_iter()
        .filter(|effect| effect.kind != SimEffectKind::ProbeForwarder)
        .collect()
}

/// Whether every context the forwarder device ever opened is released (the
/// QPROJ-8 arbiter of the device's context table).
fn no_context_open(io: &SimGuestFlowIo) -> bool {
    io.forwarder_contexts().iter().all(|context| !context.open)
}

/// The positive witness a passing phase 1 leaves (S-1: a pass that only
/// asserted "nothing happened beyond the probe" also passed against a
/// no-effect `probe_startup`): exactly one forwarder probe, run on exactly
/// one context of its own that is released, no serving context opened, and
/// nothing beyond the probe. `Err` names what is missing.
fn phase_one_witness(io: &SimGuestFlowIo) -> Result<(), String> {
    let probes = io.issued_count(SimEffectKind::ProbeForwarder);
    let opens = io.issued_count(SimEffectKind::OpenForwarder);
    let contexts = io.forwarder_contexts();
    let beyond = effects_beyond_probe(io);
    if probes == 1 && opens == 0 && contexts.len() == 1 && no_context_open(io) && beyond.is_empty()
    {
        Ok(())
    } else {
        Err(format!(
            "phase-1 witness: probes {probes} (want 1), serving opens {opens} (want 0), \
             contexts {contexts:?} (want one, released), beyond the probe {beyond:?} (want none)"
        ))
    }
}

fn arb_v4net() -> impl Strategy<Value = Ipv4Net> {
    (any::<u32>(), 8_u8..=30).prop_map(|(bits, len)| {
        Ipv4Net::new(Ipv4Addr::from(bits), len).expect("prefix length in range").trunc()
    })
}

/// A range meeting the guest prefix: one covering it (prefix length 8..=16)
/// or one inside it (17..=30).
fn arb_overlapping_v4net() -> impl Strategy<Value = Ipv4Net> {
    (any::<u16>(), 8_u8..=30).prop_map(|(host, len)| {
        let [hi, lo] = host.to_be_bytes();
        Ipv4Net::new(Ipv4Addr::new(100, 95, hi, lo), len).expect("prefix length in range").trunc()
    })
}

/// One member of a generated VIP range: half the draws meet the guest prefix,
/// so the refusing side of the iff is exercised in most cases rather than
/// rarely (a uniformly drawn range meets a `/16` with probability well under
/// 1 %, which let the property pass against a no-effect `probe_startup`).
fn arb_vip_member() -> impl Strategy<Value = Ipv4Net> {
    prop_oneof![arb_v4net(), arb_overlapping_v4net()]
}

fn overlaps(a: Ipv4Net, b: Ipv4Net) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}

// ---------------------------------------------------------------------------
// D8-PREFIX-VIP — a VIP range overlapping the guest prefix
// ---------------------------------------------------------------------------

/// P4-S14 (D8-PREFIX-VIP) — A configured VIP range overlapping the guest
/// prefix refuses startup before any boot effect, naming the range; disjoint
/// ranges pass and change nothing.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 13"]
async fn a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect() {
    for overlapping in ["100.95.255.0/24", "100.94.0.0/15"] {
        let serve = Serve::start(owner_ports(&[net(overlapping)]));
        let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");
        assert!(
            matches!(
                refused,
                GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range }
                    if prefix == GUEST_NODE_PREFIX && vip_range == net(overlapping)
            ),
            "names the VIP range {overlapping}, got {refused:?}",
        );
        assert!(
            boot_effects(&serve.io).is_empty(),
            "no boot effect before the refusal ({overlapping})"
        );
        serve.exit();
    }
    for entry in [PhaseOne::BootSeam, PhaseOne::Owner] {
        for disjoint in ["100.96.0.0/16", "10.96.0.0/16"] {
            let serve = Serve::start(owner_ports(&[net(disjoint)]));
            phase_one_over(&serve, entry, &[net(disjoint)])
                .await
                .unwrap_or_else(|e| panic!("{entry:?} {disjoint} boots: {e}"));
            phase_one_witness(&serve.io).unwrap_or_else(|w| panic!("{entry:?} {disjoint}: {w}"));
            serve.exit();
        }
    }
}

/// P4-S14 (D8-PREFIX-VIP) — `probe_startup` refuses exactly when some range of
/// the configured VIP range meets the guest prefix.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 13"]
fn probe_startup_refuses_a_vip_range_iff_one_of_its_ranges_meets_the_guest_prefix() {
    proptest!(|(ranges in proptest::collection::vec(arb_vip_member(), 1..5))| {
        prop_assume!(VipRange::new(ranges.clone(), BTreeSet::new()).is_ok());
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let expect_refusal = ranges.iter().any(|range| overlaps(*range, GUEST_NODE_PREFIX));
        for entry in [PhaseOne::BootSeam, PhaseOne::Owner] {
            let serve = Serve::start(owner_ports(&ranges));
            let outcome = runtime.block_on(phase_one_over(&serve, entry, &ranges));
            prop_assert_eq!(
                matches!(outcome, Err(GuestNetworkError::GuestPrefixOverlapsVipRange { .. })),
                expect_refusal,
                "{:?}", entry
            );
            if expect_refusal {
                prop_assert!(boot_effects(&serve.io).is_empty(), "no effect before the refusal");
            } else {
                prop_assert!(outcome.is_ok(), "{:?}: a disjoint range passes: {:?}", entry, outcome);
                let witness = phase_one_witness(&serve.io);
                prop_assert!(witness.is_ok(), "{:?}: {:?}", entry, witness);
            }
            serve.exit();
        }
    });
}

// ---------------------------------------------------------------------------
// D8-PREFIX-CFG — a node interface address inside the guest prefix
// ---------------------------------------------------------------------------

/// P4-S14 (D8-PREFIX-CFG) — A root-namespace interface address inside the
/// guest prefix refuses startup before any boot effect, naming the interface
/// and the address.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 13"]
async fn a_node_address_inside_the_prefix_refuses_before_any_boot_effect() {
    let eth0 = ifindex(2);
    let eth1 = ifindex(3);
    for inside in ["100.95.7.1/24", "100.95.255.254/32", "100.95.0.2/16"] {
        let serve = Serve::start(owner_ports(&disjoint_vip()))
            .with_interfaces(vec![(eth0, vec!["10.0.2.15/24"]), (eth1, vec![inside])]);
        let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");
        assert!(
            matches!(
                refused,
                GuestNetworkError::GuestPrefixOverlapsNodeAddress { prefix, interface, address }
                    if prefix == GUEST_NODE_PREFIX && interface == eth1 && address == net(inside)
            ),
            "names the interface and its address {inside}, got {refused:?}",
        );
        assert!(
            boot_effects(&serve.io).is_empty(),
            "no boot effect before the refusal ({inside}): no forwarder, listener or gateway route",
        );
        serve.exit();
    }
}

/// P4-S14 — A node that overlaps the prefix both by an interface address and
/// by its VIP range is refused naming the VIP range (D8-PREFIX-VIP is checked
/// first).
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 13"]
async fn a_node_that_overlaps_the_prefix_by_address_and_by_vip_range_names_the_vip_range() {
    let serve = Serve::start(owner_ports(&[net("100.95.128.0/17")]))
        .with_interfaces(vec![(ifindex(2), vec!["100.95.7.1/24"])]);
    let refused = serve.owner.probe_startup().await.expect_err("probe_startup refuses");
    assert!(
        matches!(
            refused,
            GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range }
                if prefix == GUEST_NODE_PREFIX && vip_range == net("100.95.128.0/17")
        ),
        "the VIP range is named, got {refused:?}",
    );
    assert!(boot_effects(&serve.io).is_empty(), "no boot effect before the refusal");
    serve.exit();
}

/// P4-S14 (D8-PREFIX-CFG) — Interface addresses adjacent to, or unrelated to,
/// the guest prefix pass and change nothing.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 13"]
async fn node_addresses_outside_the_prefix_pass_probe_startup_without_effect() {
    for entry in [PhaseOne::BootSeam, PhaseOne::Owner] {
        let serve = Serve::start(owner_ports(&disjoint_vip())).with_interfaces(vec![
            (ifindex(2), vec!["10.0.2.15/24"]),
            (ifindex(3), vec!["100.94.255.254/16", "100.96.0.1/16"]),
        ]);
        phase_one(&serve, entry)
            .await
            .unwrap_or_else(|e| panic!("{entry:?}: no node address in the prefix passes: {e}"));
        phase_one_witness(&serve.io).unwrap_or_else(|w| panic!("{entry:?}: {w}"));
        serve.exit();
    }
}

/// An interface address either inside the guest prefix (its address in
/// `100.95.0.0/16`, prefix length 16 or longer) or wholly disjoint from it.
fn arb_node_address() -> impl Strategy<Value = Ipv4Net> {
    prop_oneof![
        (any::<u16>(), 16_u8..=32).prop_map(|(host, len)| {
            let [hi, lo] = host.to_be_bytes();
            Ipv4Net::new(Ipv4Addr::new(100, 95, hi, lo), len).expect("prefix length in range")
        }),
        arb_v4net().prop_filter("disjoint from the guest prefix", |candidate| {
            !overlaps(*candidate, GUEST_NODE_PREFIX)
        }),
    ]
}

/// P4-S14 (D8-PREFIX-CFG) — `probe_startup` refuses exactly when some
/// interface address lies inside the guest prefix.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER step 13"]
fn probe_startup_refuses_a_node_address_iff_one_lies_inside_the_guest_prefix() {
    proptest!(|(inventory in proptest::collection::vec(proptest::collection::btree_set(arb_node_address(), 0..3), 1..4))| {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let inside = inventory
            .iter()
            .flatten()
            .any(|address| GUEST_NODE_PREFIX.contains(&address.addr()));
        for entry in [PhaseOne::BootSeam, PhaseOne::Owner] {
            let serve = Serve::start(owner_ports(&disjoint_vip()));
            serve.io.update_host_facts(|facts| {
                facts.interfaces = inventory
                    .iter()
                    .zip(2_u32..)
                    .map(|(addresses, raw)| (ifindex(raw), addresses.clone()))
                    .collect();
            });
            let outcome = runtime.block_on(phase_one(&serve, entry));
            prop_assert_eq!(
                matches!(outcome, Err(GuestNetworkError::GuestPrefixOverlapsNodeAddress { .. })),
                inside,
                "{:?}", entry
            );
            if inside {
                prop_assert!(boot_effects(&serve.io).is_empty(), "no effect before the refusal");
            } else {
                prop_assert!(outcome.is_ok(), "{:?}: a disjoint inventory passes: {:?}", entry, outcome);
                let witness = phase_one_witness(&serve.io);
                prop_assert!(witness.is_ok(), "{:?}: {:?}", entry, witness);
            }
            serve.exit();
        }
    });
}

// ---------------------------------------------------------------------------
// D24 owner placement and the fd budget
// ---------------------------------------------------------------------------

/// P4-S13 / D24 — An owner outside the `connect4` attach path
/// (`overdrive.slice`) refuses startup before any boot effect, naming its
/// cgroup.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 11"]
async fn an_owner_outside_the_connect4_attach_path_refuses_before_any_boot_effect() {
    for outside in ["/sys/fs/cgroup/system.slice", "/sys/fs/cgroup"] {
        let serve = Serve::start(owner_ports(&disjoint_vip()));
        serve.io.update_host_facts(|facts| facts.owner_cgroup = PathBuf::from(outside));
        let refused = serve.owner.probe_startup().await.expect_err("refuses OwnerCgroupPlacement");
        assert!(
            matches!(
                &refused,
                GuestNetworkError::OwnerCgroupPlacement { owner, .. }
                    if owner.as_path() == std::path::Path::new(outside)
            ),
            "OwnerCgroupPlacement naming {outside}, got {refused:?}",
        );
        assert!(
            effects_beyond_probe(&serve.io).is_empty(),
            "no boot effect beyond the probe before the refusal"
        );
        serve.exit();
    }
    let serve = Serve::start(owner_ports(&disjoint_vip()));
    serve.io.update_host_facts(|facts| {
        facts.owner_cgroup = PathBuf::from("/sys/fs/cgroup/overdrive.slice/control-plane.slice");
    });
    serve.owner.probe_startup().await.expect("an owner below the attach path passes");
    serve.exit();
}

/// P4-S13 — An `RLIMIT_NOFILE` below the owner's fd budget refuses startup
/// before any boot effect, naming what is available.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn an_rlimit_nofile_below_the_owner_budget_refuses_before_any_boot_effect() {
    let serve = Serve::start(owner_ports(&disjoint_vip()));
    serve.io.update_host_facts(|facts| facts.nofile_limit = 1_024);
    let refused = serve.owner.probe_startup().await.expect_err("refuses FdBudget");
    assert!(
        matches!(refused, GuestNetworkError::FdBudget { required, available: 1_024 } if required > 1_024),
        "FdBudget naming RLIMIT_NOFILE, got {refused:?}",
    );
    assert!(
        effects_beyond_probe(&serve.io).is_empty(),
        "no boot effect beyond the probe before the refusal"
    );
    serve.exit();
}

// ---------------------------------------------------------------------------
// G-V1 / D-8 — the forwarder probe at boot step 1, on a context of its own
// ---------------------------------------------------------------------------

/// One scripted forwarder probe failure with the typed cause its refusal
/// must carry.
type ForwarderFailureCase = (&'static str, SimFailure, fn(&ForwarderError) -> bool);

/// Which phase-1 entry point a case drives.
#[derive(Debug, Clone, Copy)]
enum PhaseOne {
    /// `probe_guest_flow_startup` over the boot's seam, before any owner.
    BootSeam,
    /// The owner's `probe_startup` (the same function over its own seam).
    Owner,
}

async fn phase_one(serve: &Serve, entry: PhaseOne) -> Result<(), GuestNetworkError> {
    phase_one_over(serve, entry, &disjoint_vip()).await
}

/// Phase 1 through `entry`, where the boot-seam entry is given `vip` — the
/// same ranges the owner of `serve` was built with, so the two entry points
/// check one configuration.
async fn phase_one_over(
    serve: &Serve,
    entry: PhaseOne,
    vip: &[Ipv4Net],
) -> Result<(), GuestNetworkError> {
    match entry {
        PhaseOne::BootSeam => {
            let vip = VipRange::new(vip.to_vec(), BTreeSet::new()).expect("valid VIP range");
            let seam: &dyn GuestFlowIo = &*serve.io;
            probe_guest_flow_startup(seam, &vip, GUEST_NODE_PREFIX).await
        }
        PhaseOne::Owner => serve.owner.probe_startup().await,
    }
}

/// G-V1 / D-8 (ADR-0205, ADR-0159, EVMOD-CTL) — phase 1 runs the forwarder
/// probe on a context it opens and releases itself, after the four
/// configuration checks and before any boot effect. A missing, mis-built,
/// unloadable or misbehaving module refuses with the forwarder's typed
/// cause and leaves nothing behind; a good forwarder passes, also leaving no
/// context open. Phase 1 never opens the serving context.
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given a node whose configuration passes every other check. When the
/// forwarder probe is scripted to fail `DeviceAbsent`, `AbiMismatch` or a
/// stage mismatch (`Backpressure` observing
/// `SenderNotBlocked`; `Cleanup` observing residue), then phase 1 — through
/// `probe_guest_flow_startup` and through the owner's `probe_startup` —
/// returns `GuestNetworkError::Forwarder` naming that cause, every
/// forwarder context is released, and no serving context, listener,
/// gateway route or other boot effect was issued. With a good forwarder,
/// phase 1 passes after exactly one forwarder probe, on one context that is
/// released before it returns.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one() {
    let cases: Vec<ForwarderFailureCase> = vec![
        ("DeviceAbsent", SimFailure::Forwarder(SimForwarderFailure::DeviceAbsent), |e| {
            matches!(e, ForwarderError::DeviceAbsent { .. })
        }),
        ("AbiMismatch", SimFailure::Forwarder(SimForwarderFailure::AbiMismatch), |e| {
            matches!(e, ForwarderError::AbiMismatch { expected: 1, found: 2 })
        }),
        (
            "Backpressure stage",
            SimFailure::Forwarder(SimForwarderFailure::ProbeMismatch(
                ForwarderProbeStage::Backpressure,
                ForwarderProbeObservation::SenderNotBlocked,
            )),
            |e| {
                matches!(
                    e,
                    ForwarderError::ProbeMismatch {
                        stage: ForwarderProbeStage::Backpressure,
                        observed: ForwarderProbeObservation::SenderNotBlocked,
                    }
                )
            },
        ),
        (
            "Cleanup stage",
            SimFailure::Forwarder(SimForwarderFailure::ProbeMismatch(
                ForwarderProbeStage::Cleanup,
                ForwarderProbeObservation::ResidueRemains { pairs: 1 },
            )),
            |e| {
                matches!(
                    e,
                    ForwarderError::ProbeMismatch {
                        stage: ForwarderProbeStage::Cleanup,
                        observed: ForwarderProbeObservation::ResidueRemains { pairs: 1 },
                    }
                )
            },
        ),
    ];
    for entry in [PhaseOne::BootSeam, PhaseOne::Owner] {
        for (name, failure, is_cause) in &cases {
            let serve = Serve::start(owner_ports(&disjoint_vip()));
            serve.io.script_failure(SimEffectKind::ProbeForwarder, *failure);
            match phase_one(&serve, entry).await {
                Err(GuestNetworkError::Forwarder { source }) if is_cause(&source) => {}
                other => panic!(
                    "{entry:?} {name}: phase 1 refuses with the forwarder's cause, got {other:?}"
                ),
            }
            assert!(
                no_context_open(&serve.io),
                "{entry:?} {name}: no forwarder context is left open: {:?}",
                serve.io.forwarder_contexts()
            );
            assert!(
                effects_beyond_probe(&serve.io).is_empty(),
                "{entry:?} {name}: nothing follows the failed probe: {:?}",
                boot_effects(&serve.io)
            );
            serve.exit();
        }

        let serve = Serve::start(owner_ports(&disjoint_vip()));
        phase_one(&serve, entry)
            .await
            .unwrap_or_else(|e| panic!("{entry:?}: a good forwarder passes: {e}"));
        assert_eq!(
            serve.io.issued_count(SimEffectKind::ProbeForwarder),
            1,
            "{entry:?}: one forwarder probe at step 1"
        );
        assert_eq!(
            serve.io.issued_count(SimEffectKind::OpenForwarder),
            0,
            "{entry:?}: phase 1 does not open the serving context (that is boot step 9)"
        );
        let contexts = serve.io.forwarder_contexts();
        assert_eq!(contexts.len(), 1, "{entry:?}: the probe ran on one context of its own");
        assert!(
            no_context_open(&serve.io),
            "{entry:?}: the probe's context is released: {contexts:?}"
        );
        assert!(effects_beyond_probe(&serve.io).is_empty(), "{entry:?}: nothing beyond the probe");
        serve.exit();
    }
}
