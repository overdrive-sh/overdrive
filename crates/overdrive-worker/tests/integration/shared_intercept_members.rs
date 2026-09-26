//! GH #295 — Lima-root, real-nft evidence for the dynamic members of the
//! node-shared intercept program (S-ND295-54, the E8 Lima half; S-ND295-61,
//! the E13 Lima half).
//!
//! - **S-ND295-54** drives `MtlsIntercept::remove_allocation_elements` on the
//!   production `HostMtlsIntercept` against a real program with a real
//!   pre-absent member (feature delta § *Driven port — intercept element
//!   release, member convergence, boot clear*, `remove_allocation_elements`).
//! - **S-ND295-61** drives the worker's `audit_shared_owner` and
//!   `converge_shared_owner` over `HostMtlsIntercept` with live allocations
//!   (non-empty member sets) and deletes one intercept object at a time: one
//!   member, the whole program table, the `fwmark 0x1 lookup 100` rule,
//!   table 100's local route, and (R18-conditional) the intercept-mark guard
//!   table (feature delta *Runtime member audit (R15)* and *Runtime repair
//!   contract*). The port's `observe_shared_state` and
//!   `Client::local_route_present` observe the loss and the repair.
//!
//! Faults enter only as real kernel mutations (`nft delete element`,
//! `nft delete table`, `ip rule del`, `ip route del`); the diagnostic host
//! tools also confirm each loss happened before the port is asked about it.
//! No fixture writes a state an oracle reads.
//!
//! # Lane and isolation
//!
//! Integration, Lima + root: real `nft`, real policy routing, and
//! `IP_TRANSPARENT` listeners. Every body holds the cross-process kernel-state
//! lock the sibling suites hold, and the whole `overdrive-worker` integration
//! binary is in the `host-kernel-shared` nextest group. A [`MemberSandbox`]
//! scrubs the node-global intercept state at construction and on drop, so a
//! panic leaves nothing behind. Evidence is append-only: one stderr line per
//! observation, stamped with the scenario, the row, the phase, and the elapsed
//! time; cleanup records its own lines and never replaces an earlier one.
//!
//! Addresses: guest members come from `100.95.54.0/24` and `100.95.61.0/24`.
//! They are only set elements — no interface, address, or route carries them —
//! and they are disjoint from production's `10.99.0.0/16`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "Tier-3 bodies: evidence lines go to stderr; fixture preconditions and contract \
              violations must panic with informative messages; each body is one composed \
              real-kernel scenario"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, GuestNetworkAssignment, Resources, VmPayload,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::{AllocationId, SpiffeId};
use overdrive_netlink::nft::{self, SharedIpInterceptIdentity};
use overdrive_netlink::{Client, block_on_host_netlink};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
use overdrive_worker::mtls_intercept::InterceptError;
use overdrive_worker::mtls_intercept_port::{
    HostMtlsIntercept, InterceptMembers, InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptWorker, MtlsSharedOwnerError};

use super::inbound_tproxy_harness::{KernelStateLock, is_root, record_uname};
use super::leg_listener::LegListener;

/// The owned program's table, in the `ip` family.
const OWNED_TABLE: &str = "overdrive-mtls";
/// The D-295-R18 intercept-mark guard table (R18-conditional).
const GUARD_TABLE: &str = "overdrive-mtls-guard";
/// A foreign table whose bytes a removal must leave unchanged.
const FOREIGN_TABLE: &str = "nd295_members_foreign";
/// The policy route's fwmark and routing table.
const POLICY_FWMARK: u32 = 0x1;
const POLICY_TABLE: u32 = 100;

// ---------------------------------------------------------------------------
// Append-only evidence
// ---------------------------------------------------------------------------

/// Append-only evidence: every observation is one new stderr line stamped with
/// the scenario, the row, the phase, and the elapsed time since the body began.
#[derive(Clone, Copy)]
struct Evidence {
    scenario: &'static str,
    started: Instant,
}

impl Evidence {
    fn begin(scenario: &'static str) -> Self {
        Self { scenario, started: Instant::now() }
    }

    fn record(&self, row: &str, phase: &str, detail: impl std::fmt::Display) {
        eprintln!(
            "[{}][{row}][{phase}][+{}ms] {detail}",
            self.scenario,
            self.started.elapsed().as_millis()
        );
    }
}

/// Run one diagnostic host command and record its exit status and output.
fn run_recorded(evidence: Evidence, row: &str, phase: &str, program: &str, args: &[&str]) -> bool {
    match Command::new(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
        Ok(output) => {
            evidence.record(
                row,
                phase,
                format_args!(
                    "{program} {args:?} -> {:?} stdout={:?} stderr={:?}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            );
            output.status.success()
        }
        Err(error) => {
            evidence.record(
                row,
                phase,
                format_args!("{program} {args:?} could not spawn: {error}"),
            );
            false
        }
    }
}

/// Apply one real kernel mutation; it must succeed, or the fault never happened.
fn kernel_mutation(evidence: Evidence, row: &str, program: &str, args: &[&str]) {
    assert!(
        run_recorded(evidence, row, "fault", program, args),
        "[{row}] the kernel mutation `{program} {args:?}` must succeed"
    );
}

/// The standard output of one diagnostic host command, which must succeed.
fn command_stdout(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("spawn `{program} {args:?}`: {error}"));
    assert!(
        output.status.success(),
        "`{program} {args:?}` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// ---------------------------------------------------------------------------
// Node-global intercept sandbox
// ---------------------------------------------------------------------------

/// Scrubs every node-global object these bodies can create — both
/// `overdrive-mtls` families, the guard table, the foreign sentinel, every
/// `fwmark 0x1 lookup 100` rule, and table 100's local route — when created
/// and again when dropped. Each step is recorded; a step that finds nothing to
/// remove is the "already clean" outcome, not a failure.
struct MemberSandbox {
    evidence: Evidence,
    row: &'static str,
}

impl MemberSandbox {
    fn fresh(evidence: Evidence, row: &'static str) -> Self {
        scrub(evidence, row, "setup-scrub");
        Self { evidence, row }
    }
}

impl Drop for MemberSandbox {
    fn drop(&mut self) {
        scrub(self.evidence, self.row, "cleanup-scrub");
    }
}

fn scrub(evidence: Evidence, row: &str, phase: &str) {
    for (family, table) in
        [("ip", OWNED_TABLE), ("bridge", OWNED_TABLE), ("ip", GUARD_TABLE), ("ip", FOREIGN_TABLE)]
    {
        run_recorded(evidence, row, phase, "nft", &["delete", "table", family, table]);
    }
    // A healthy run leaves one fwmark rule; drain however many an earlier
    // interrupted run stacked.
    for _ in 0..64 {
        if !run_recorded(
            evidence,
            row,
            phase,
            "ip",
            &["rule", "del", "fwmark", "0x1", "lookup", "100"],
        ) {
            break;
        }
    }
    run_recorded(
        evidence,
        row,
        phase,
        "ip",
        &["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"],
    );
}

fn create_foreign_sentinel(evidence: Evidence, row: &str) {
    kernel_mutation(evidence, row, "nft", &["add", "table", "ip", FOREIGN_TABLE]);
    kernel_mutation(evidence, row, "nft", &["add", "chain", "ip", FOREIGN_TABLE, "sentinel"]);
    kernel_mutation(
        evidence,
        row,
        "nft",
        &["add", "rule", "ip", FOREIGN_TABLE, "sentinel", "counter", "accept"],
    );
}

/// The JSON listing of one nft table, or `None` when it is absent.
fn nft_table_json(family: &str, table: &str) -> Option<Vec<u8>> {
    let output = Command::new("nft")
        .args(["-j", "-a", "list", "table", family, table])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn an nft JSON table listing");
    output.status.success().then_some(output.stdout)
}

/// The bytes of every table the intercept owns, members included.
fn owned_tables_snapshot() -> Vec<(&'static str, &'static str, Option<Vec<u8>>)> {
    [("ip", OWNED_TABLE), ("bridge", OWNED_TABLE), ("ip", GUARD_TABLE)]
        .into_iter()
        .map(|(family, table)| (family, table, nft_table_json(family, table)))
        .collect()
}

/// The owned program's rules, without their counters: a changed handle,
/// userdata, or program means the program was rewritten.
fn owned_rules() -> Vec<(&'static str, u64, Vec<u8>, Vec<u8>)> {
    ["prerouting", "output"]
        .into_iter()
        .flat_map(|chain| {
            nft::list_rules(OWNED_TABLE, chain)
                .unwrap_or_else(|error| panic!("list the owned {chain} rules: {error}"))
                .into_iter()
                .map(move |rule| (chain, rule.handle, rule.userdata, rule.normalized_program))
        })
        .collect()
}

/// The independent netlink read-back of the owned program and its members.
fn kernel_members() -> (SharedIpInterceptIdentity, InterceptMembers) {
    let state = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds")
        .expect("the owned shared-IP program is present");
    let members = InterceptMembers {
        managed_guest_ips: state.managed_guest_ips().clone(),
        outbound_sources: state.outbound_sources().clone(),
        inbound_destinations: state.inbound_destinations().clone(),
    };
    (state.identity().clone(), members)
}

/// Whether the fwmark rule and table 100's local route are present, read
/// through netlink (`Client::fib_rule_fwmark_present`,
/// `Client::local_route_present`).
fn policy_route_parts() -> (bool, bool) {
    block_on_host_netlink(|| async {
        let client = Client::new()?;
        let fwmark = client.fib_rule_fwmark_present(POLICY_FWMARK, POLICY_TABLE).await?;
        let route = client.local_route_present(POLICY_TABLE, "lo").await?;
        Ok((fwmark, route))
    })
    .expect("the policy route is readable through netlink")
}

fn is_fwmark_rule(line: &str) -> bool {
    (line.contains("fwmark 0x1 ") || line.ends_with("fwmark 0x1") || line.contains("fwmark 1 "))
        && line.contains("lookup 100")
}

/// `ip rule show`, split into the owned fwmark rules and their complement.
fn rpdb() -> (usize, Vec<String>) {
    let listing = command_stdout("ip", &["rule", "show"]);
    let owned = listing.lines().filter(|line| is_fwmark_rule(line)).count();
    let complement =
        listing.lines().filter(|line| !is_fwmark_rule(line)).map(str::to_owned).collect::<Vec<_>>();
    (owned, complement)
}

fn is_owned_local_route(line: &str) -> bool {
    line.starts_with("local default") && line.contains("dev lo")
}

/// `ip route show table 100`, split into the owned local route and its
/// complement.
fn table_100() -> (usize, Vec<String>) {
    let listing = command_stdout("ip", &["route", "show", "table", "100"]);
    let owned = listing.lines().filter(|line| is_owned_local_route(line)).count();
    let complement = listing
        .lines()
        .filter(|line| !is_owned_local_route(line))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (owned, complement)
}

// ---------------------------------------------------------------------------
// S-ND295-54 — convergent removal on the real kernel
// ---------------------------------------------------------------------------

/// The stopping allocation's source address.
const REMOVAL_STOPPING_SOURCE: Ipv4Addr = Ipv4Addr::new(100, 95, 54, 2);
/// A requested destination that is present when removal runs.
const REMOVAL_STOPPING_PRESENT: SocketAddrV4 = SocketAddrV4::new(REMOVAL_STOPPING_SOURCE, 8080);
/// A requested destination deleted out of band before removal runs.
const REMOVAL_STOPPING_PRE_ABSENT: SocketAddrV4 = SocketAddrV4::new(REMOVAL_STOPPING_SOURCE, 8443);
/// A second, surviving allocation whose members must be left unchanged.
const REMOVAL_SURVIVING_SOURCE: Ipv4Addr = Ipv4Addr::new(100, 95, 54, 3);
const REMOVAL_SURVIVING_DESTINATION: SocketAddrV4 =
    SocketAddrV4::new(REMOVAL_SURVIVING_SOURCE, 8443);

/// Removal on the real kernel is convergent, and every refused request leaves
/// the owned tables byte-equal (feature delta `remove_allocation_elements`,
/// FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (`remove_allocation_elements` through the netlink surface); E8's Lima column).
///
/// A real program at two listener targets holds a stopping allocation (one
/// source, two requested destinations) and a surviving allocation. One of the
/// stopping allocation's destinations is deleted out of band with
/// `nft delete element`, so it is absent before removal runs. Then:
///
/// 1. two rejected batches — a duplicate destination and a zero-port
///    destination — each return `Err` and leave every owned table byte-equal;
/// 2. an adapter that has recorded no program (a fresh `HostMtlsIntercept`
///    whose `converge_shared` never ran) is refused with
///    `SharedProgramNotConverged` (DISTILL gap B-8, removal refusal 2, after
///    argument validation), and the tables stay byte-equal. The
///    recorded-versus-observed refusal has its own body,
///    [`removal_is_refused_when_the_recorded_program_was_replaced_out_of_band`];
/// 3. the real removal returns `Ok(InterceptState)` without every requested
///    member — the pre-absent one included, which is not an error — with the
///    surviving allocation's members, the program, and the policy route
///    unchanged, and a foreign table byte-equal; an independent netlink
///    read-back agrees;
/// 4. the removal retired the stopping allocation's element tokens: dropping
///    its guards changes nothing.
///
/// A commit the kernel itself rejects, and a post-commit read-back failure,
/// cannot be produced on demand against a real kernel without racing the
/// adapter's observation; the source-local scripted seam
/// (`mtls_intercept_port::shared_program_rollback_acceptance::remove_allocation_elements_deletes_only_present_requested_members`)
/// owns those two arms.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-54 — Protection removal is convergent and its failures are typed.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 07-01 (S-ND295-54)"]
fn convergent_removal_with_a_pre_absent_member_and_batch_rejection_preserves_state() {
    assert!(is_root(), "S-ND295-54 Lima evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-54");
    let evidence = Evidence::begin("S-ND295-54");
    let row = "convergent-removal";
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = MemberSandbox::fresh(evidence, row);
    create_foreign_sentinel(evidence, row);
    let foreign_before = nft_table_json("ip", FOREIGN_TABLE).expect("the foreign sentinel exists");

    let host = HostMtlsIntercept::new();
    let leg_f = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg F");
    let leg_c = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg C");
    let outbound_target = leg_f.bound_v4().expect("leg F reports its bound IPv4 address");
    let inbound_target = leg_c.bound_v4().expect("leg C reports its bound IPv4 address");
    let node_guard = host
        .converge_shared(None, outbound_target, inbound_target)
        .expect("the owned program installs on a clean kernel");
    let program = host
        .observe_shared()
        .expect("the empty program is observable")
        .expect("the owned program is present");
    let stopping_outbound = host
        .install_outbound(REMOVAL_STOPPING_SOURCE, outbound_target.port())
        .expect("install the stopping allocation's source members");
    let stopping_present = host
        .install_inbound(REMOVAL_STOPPING_PRESENT, inbound_target.port())
        .expect("install the stopping allocation's present destination");
    let stopping_pre_absent = host
        .install_inbound(REMOVAL_STOPPING_PRE_ABSENT, inbound_target.port())
        .expect("install the destination that will be pre-absent");
    let surviving_outbound = host
        .install_outbound(REMOVAL_SURVIVING_SOURCE, outbound_target.port())
        .expect("install the surviving allocation's source members");
    let surviving_destination = host
        .install_inbound(REMOVAL_SURVIVING_DESTINATION, inbound_target.port())
        .expect("install the surviving allocation's destination");

    // The real pre-absent member.
    kernel_mutation(
        evidence,
        row,
        "nft",
        &[
            "delete",
            "element",
            "ip",
            OWNED_TABLE,
            "inbound_destinations",
            &format!(
                "{{ {} . {} }}",
                REMOVAL_STOPPING_PRE_ABSENT.ip(),
                REMOVAL_STOPPING_PRE_ABSENT.port()
            ),
        ],
    );
    let (identity, before_members) = kernel_members();
    evidence.record(row, "pre-state", format_args!("{before_members:?}"));
    assert!(
        !before_members.inbound_destinations.contains(&REMOVAL_STOPPING_PRE_ABSENT)
            && before_members.inbound_destinations.contains(&REMOVAL_STOPPING_PRESENT),
        "the pre-absent destination is absent and the present one present before removal"
    );
    let before = owned_tables_snapshot();

    // 1. Rejected batches preserve the pre-state.
    for (batch, destinations) in [
        ("duplicate-destination", vec![REMOVAL_STOPPING_PRESENT, REMOVAL_STOPPING_PRESENT]),
        (
            "zero-port-destination",
            vec![REMOVAL_STOPPING_PRESENT, SocketAddrV4::new(REMOVAL_STOPPING_SOURCE, 0)],
        ),
    ] {
        let refused = host.remove_allocation_elements(REMOVAL_STOPPING_SOURCE, &destinations);
        evidence.record(batch, "refusal", format_args!("{refused:?}"));
        assert!(refused.is_err(), "[{batch}] an invalid destination batch is refused");
        assert_eq!(
            owned_tables_snapshot(),
            before,
            "[{batch}] a rejected removal batch leaves every owned table byte-equal"
        );
    }

    // 2. An adapter that recorded no program is refused by the precondition.
    let unrecorded = HostMtlsIntercept::new();
    let refused = unrecorded.remove_allocation_elements(
        REMOVAL_STOPPING_SOURCE,
        &[REMOVAL_STOPPING_PRESENT, REMOVAL_STOPPING_PRE_ABSENT],
    );
    evidence.record("not-converged", "refusal", format_args!("{refused:?}"));
    assert!(
        matches!(refused, Err(InterceptError::SharedProgramNotConverged)),
        "an adapter whose converge_shared never ran is refused with SharedProgramNotConverged, \
         got {refused:?}"
    );
    assert_eq!(
        owned_tables_snapshot(),
        before,
        "a precondition refusal leaves every owned table byte-equal"
    );

    // 3. The convergent removal.
    let removed = host
        .remove_allocation_elements(
            REMOVAL_STOPPING_SOURCE,
            &[REMOVAL_STOPPING_PRESENT, REMOVAL_STOPPING_PRE_ABSENT],
        )
        .expect("removal converges when a requested member is already absent");
    evidence.record(row, "removed", format_args!("{removed:?}"));
    let surviving = InterceptMembers {
        managed_guest_ips: BTreeSet::from([REMOVAL_SURVIVING_SOURCE]),
        outbound_sources: BTreeSet::from([REMOVAL_SURVIVING_SOURCE]),
        inbound_destinations: BTreeSet::from([REMOVAL_SURVIVING_DESTINATION]),
    };
    assert_eq!(
        removed.members, surviving,
        "every requested member is absent and every other member is unchanged"
    );
    assert_eq!(removed.program, program, "the owned program is unchanged");
    assert!(removed.policy_route, "the policy route is unchanged");
    let (identity_after, members_after) = kernel_members();
    assert_eq!(identity_after, identity, "the kernel program identity is unchanged");
    assert_eq!(members_after, surviving, "the kernel read-back agrees with the returned state");
    assert_eq!(
        nft_table_json("ip", FOREIGN_TABLE).expect("the foreign sentinel survives"),
        foreign_before,
        "the foreign complement is byte-equal"
    );

    // 4. The removal retired the stopping allocation's element tokens.
    let after_removal = owned_tables_snapshot();
    drop(stopping_pre_absent);
    drop(stopping_present);
    drop(stopping_outbound);
    assert_eq!(
        owned_tables_snapshot(),
        after_removal,
        "dropping the removed allocation's guards performs no effect"
    );

    drop(surviving_destination);
    drop(surviving_outbound);
    drop(node_guard);
    drop((leg_c, leg_f));
}

/// The recorded-versus-observed branch of `remove_allocation_elements` (R10's
/// removal refusal 3, DISTILL gap B-8): a converged adapter whose kernel program
/// was replaced out of band is refused, with a typed refusal that is not the
/// precondition's, and every owned table stays byte-equal.
///
/// The adapter under test converges its program and installs one allocation.
/// The owned program table is then deleted with `nft delete table` (a real
/// kernel mutation), and a second, independent `HostMtlsIntercept` — a writer
/// the first adapter does not know about — converges a program at two other
/// listener targets. The first adapter's recorded identity now differs from the
/// observed program. Its removal must refuse without mutating, and must not be
/// the no-record refusal: the record is present, it is stale.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-54 — Protection removal is convergent and its failures are typed.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 07-01 (S-ND295-54)"]
fn removal_is_refused_when_the_recorded_program_was_replaced_out_of_band() {
    assert!(is_root(), "S-ND295-54 Lima evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-54-recorded-vs-observed");
    let evidence = Evidence::begin("S-ND295-54");
    let row = "recorded-vs-observed";
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = MemberSandbox::fresh(evidence, row);

    let host = HostMtlsIntercept::new();
    let leg_f = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg F");
    let leg_c = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg C");
    let outbound_target = leg_f.bound_v4().expect("leg F reports its bound IPv4 address");
    let inbound_target = leg_c.bound_v4().expect("leg C reports its bound IPv4 address");
    let node_guard = host
        .converge_shared(None, outbound_target, inbound_target)
        .expect("the owned program installs on a clean kernel");
    let recorded = host
        .observe_shared()
        .expect("the recorded program is observable")
        .expect("the owned program is present");
    let stopping_outbound = host
        .install_outbound(REMOVAL_STOPPING_SOURCE, outbound_target.port())
        .expect("install the stopping allocation's source members");

    // The out-of-band replacement: the table goes, and another writer converges
    // a program at two other targets.
    kernel_mutation(evidence, row, "nft", &["delete", "table", "ip", OWNED_TABLE]);
    let successor = HostMtlsIntercept::new();
    let successor_f = successor
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("the other writer binds its leg F");
    let successor_c = successor
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("the other writer binds its leg C");
    let successor_guard = successor
        .converge_shared(
            None,
            successor_f.bound_v4().expect("the other writer's leg F address"),
            successor_c.bound_v4().expect("the other writer's leg C address"),
        )
        .expect("the other writer converges its program on the emptied kernel");
    let replaced = host
        .observe_shared()
        .expect("the replaced program is observable")
        .expect("a program is present");
    assert_ne!(replaced, recorded, "the observed program differs from the recorded one");
    let before = owned_tables_snapshot();

    let refused =
        host.remove_allocation_elements(REMOVAL_STOPPING_SOURCE, &[REMOVAL_STOPPING_PRESENT]);
    evidence.record(row, "refusal", format_args!("{refused:?}"));
    assert!(refused.is_err(), "a stale recorded identity refuses the removal");
    assert!(
        !matches!(refused, Err(InterceptError::SharedProgramNotConverged)),
        "a present but stale record is the recorded-versus-observed refusal, not the \
         precondition's"
    );
    assert_eq!(
        owned_tables_snapshot(),
        before,
        "a recorded-versus-observed refusal leaves every owned table byte-equal"
    );

    drop(stopping_outbound);
    drop(node_guard);
    drop(successor_guard);
    drop((leg_c, leg_f, successor_c, successor_f));
}

// ---------------------------------------------------------------------------
// S-ND295-61 — audit and repair of each deleted intercept object
// ---------------------------------------------------------------------------

/// The one declared Service port every member allocation carries.
const MEMBER_SERVICE_PORT: u16 = 8443;
const FIRST_GUEST: Ipv4Addr = Ipv4Addr::new(100, 95, 61, 2);
const SECOND_GUEST: Ipv4Addr = Ipv4Addr::new(100, 95, 61, 3);
/// The allocation started after repair, proving the recorded targets survived.
const THIRD_GUEST: Ipv4Addr = Ipv4Addr::new(100, 95, 61, 4);

/// One deleted intercept object (E13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterceptLoss {
    /// One member of a live allocation: its inbound destination.
    Member,
    /// The whole owned program table, members included.
    ProgramTable,
    /// The `fwmark 0x1 lookup 100` routing rule.
    FwmarkRule,
    /// Table 100's `local 0.0.0.0/0 dev lo` route.
    LocalRoute,
    /// The D-295-R18 intercept-mark guard table. R18-conditional: DELIVER step
    /// 08-01 removes this variant with the guard body if R18 is withdrawn.
    MarkGuardTable,
}

impl InterceptLoss {
    const fn row(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::ProgramTable => "program-table",
            Self::FwmarkRule => "fwmark-rule",
            Self::LocalRoute => "local-route",
            Self::MarkGuardTable => "mark-guard-table",
        }
    }

    const fn component(self) -> SharedGuestNetworkComponent {
        match self {
            Self::Member => SharedGuestNetworkComponent::IpSets,
            Self::ProgramTable | Self::FwmarkRule | Self::LocalRoute | Self::MarkGuardTable => {
                SharedGuestNetworkComponent::IpRules
            }
        }
    }

    /// Delete the object with a real kernel mutation, then confirm it is gone
    /// with a diagnostic host tool.
    fn inflict(self, evidence: Evidence) {
        let row = self.row();
        match self {
            Self::Member => {
                kernel_mutation(
                    evidence,
                    row,
                    "nft",
                    &[
                        "delete",
                        "element",
                        "ip",
                        OWNED_TABLE,
                        "inbound_destinations",
                        &format!("{{ {FIRST_GUEST} . {MEMBER_SERVICE_PORT} }}"),
                    ],
                );
                let (_, members) = kernel_members();
                assert!(
                    !members.inbound_destinations.contains(&first_destination()),
                    "the deleted member is gone"
                );
            }
            Self::ProgramTable => {
                kernel_mutation(evidence, row, "nft", &["delete", "table", "ip", OWNED_TABLE]);
                assert!(nft_table_json("ip", OWNED_TABLE).is_none(), "the owned table is gone");
            }
            Self::FwmarkRule => {
                kernel_mutation(
                    evidence,
                    row,
                    "ip",
                    &["rule", "del", "fwmark", "0x1", "lookup", "100"],
                );
                assert_eq!(rpdb().0, 0, "no fwmark 0x1 lookup 100 rule remains");
            }
            Self::LocalRoute => {
                kernel_mutation(
                    evidence,
                    row,
                    "ip",
                    &["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"],
                );
                assert_eq!(table_100().0, 0, "table 100 holds no local default route");
            }
            Self::MarkGuardTable => {
                kernel_mutation(evidence, row, "nft", &["delete", "table", "ip", GUARD_TABLE]);
                assert!(nft_table_json("ip", GUARD_TABLE).is_none(), "the guard table is gone");
            }
        }
    }

    /// What the port's observation reports once the object is lost.
    fn observed_loss(self, baseline: &InterceptState) -> Option<InterceptState> {
        match self {
            Self::Member => {
                let mut members = baseline.members.clone();
                members.inbound_destinations.remove(&first_destination());
                Some(InterceptState { members, ..baseline.clone() })
            }
            Self::ProgramTable => None,
            Self::FwmarkRule | Self::LocalRoute => {
                Some(InterceptState { policy_route: false, ..baseline.clone() })
            }
            Self::MarkGuardTable => {
                Some(InterceptState { intercept_mark_guard: false, ..baseline.clone() })
            }
        }
    }
}

const fn first_destination() -> SocketAddrV4 {
    SocketAddrV4::new(FIRST_GUEST, MEMBER_SERVICE_PORT)
}

fn member_spec(alloc: &str, address: Ipv4Addr) -> AllocationSpec {
    let octets = address.octets();
    AllocationSpec {
        alloc: AllocationId::new(alloc).expect("a valid allocation id"),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/sa/alloc/01")
            .expect("a valid SPIFFE id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/nonexistent/kernel".into(),
            rootfs: "/nonexistent/rootfs".into(),
        }),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        network: Some(GuestNetworkAssignment {
            address,
            tap: format!("nd61tap-{}", octets[3]),
            mac: [0x02, 0x00, octets[0], octets[1], octets[2], octets[3]],
            gateway: Ipv4Addr::new(100, 95, 61, 1),
            prefix: 24,
            dns: Ipv4Addr::new(100, 95, 61, 1),
        }),
        service_ports: vec![NonZeroU16::new(MEMBER_SERVICE_PORT).expect("a non-zero port")],
    }
}

fn members_of(addresses: &[Ipv4Addr]) -> InterceptMembers {
    InterceptMembers {
        managed_guest_ips: addresses.iter().copied().collect(),
        outbound_sources: addresses.iter().copied().collect(),
        inbound_destinations: addresses
            .iter()
            .map(|address| SocketAddrV4::new(*address, MEMBER_SERVICE_PORT))
            .collect(),
    }
}

fn member_worker(intercept: Arc<dyn MtlsIntercept>) -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

/// One E13 row on a freshly scrubbed kernel: start the owner and two live
/// allocations, delete `loss`, and require the audit to report exactly its
/// component within a second, the repair to restore exactly the deleted object
/// without rewriting the program, and a new allocation to install afterwards.
async fn lose_and_repair(evidence: Evidence, loss: InterceptLoss) {
    let row = loss.row();
    let _sandbox = MemberSandbox::fresh(evidence, row);
    let host = HostMtlsIntercept::new();
    let worker = member_worker(Arc::new(host.clone()));

    worker.start_shared_owner().await.expect("the shared owner publishes on a clean kernel");
    let first = member_spec("nd61-first", FIRST_GUEST);
    let second = member_spec("nd61-second", SECOND_GUEST);
    worker.start_alloc(&first).await.expect("the first live allocation installs its members");
    worker.start_alloc(&second).await.expect("the second live allocation installs its members");
    worker.audit_shared_owner().await.expect("a healthy owner with live allocations audits clean");

    let baseline = host
        .observe_shared_state()
        .expect("the healthy intercept state is observable")
        .expect("the owned program is present");
    evidence.record(row, "baseline", format_args!("{baseline:?}"));
    assert_eq!(
        baseline.members,
        members_of(&[FIRST_GUEST, SECOND_GUEST]),
        "the live allocations hold exactly their members"
    );
    assert!(baseline.policy_route, "the healthy policy route is present");
    let rules_before = owned_rules();
    let (fwmark_rules_before, rpdb_complement_before) = rpdb();
    let (local_routes_before, table_100_complement_before) = table_100();
    assert_eq!((fwmark_rules_before, local_routes_before), (1, 1), "one owned rule and route");

    loss.inflict(evidence);
    let observed = host.observe_shared_state().expect("a lost object is a typed observation");
    evidence.record(row, "observed-loss", format_args!("{observed:?}"));
    assert_eq!(
        observed,
        loss.observed_loss(&baseline),
        "the port observes exactly the deleted object as lost"
    );
    if loss == InterceptLoss::LocalRoute {
        assert_eq!(policy_route_parts(), (true, false), "only the local route is lost");
    }

    let audit_started = Instant::now();
    let detected = worker.audit_shared_owner().await;
    let detection = audit_started.elapsed();
    evidence.record(row, "detection", format_args!("{detected:?} after {detection:?}"));
    let detected = detected.expect_err("the audit reports the lost object");
    assert!(detection < Duration::from_secs(1), "the loss is detected within a second");
    assert_eq!(detected.component(), loss.component(), "the audit names the lost component");
    if loss == InterceptLoss::Member {
        let after_loss = loss.observed_loss(&baseline).expect("a member loss keeps the program");
        assert!(
            matches!(
                &detected,
                MtlsSharedOwnerError::MemberMismatch { expected, observed }
                    if *expected == baseline.members && *observed == after_loss.members
            ),
            "a member loss is a MemberMismatch naming the registry and the kernel members"
        );
    } else {
        assert!(
            matches!(detected, MtlsSharedOwnerError::Intercept { .. }),
            "a program, policy-route, or guard loss is an Intercept failure"
        );
    }

    worker.converge_shared_owner().await.expect("repair converges with live allocations");
    worker.audit_shared_owner().await.expect("the repaired owner audits clean");
    let repaired = host
        .observe_shared_state()
        .expect("the repaired intercept state is observable")
        .expect("the owned program is present after repair");
    evidence.record(row, "repaired", format_args!("{repaired:?}"));
    assert_eq!(repaired, baseline, "repair restores exactly the deleted object");
    assert_eq!(policy_route_parts(), (true, true), "the fwmark rule and local route are present");
    let (fwmark_rules_after, rpdb_complement_after) = rpdb();
    let (local_routes_after, table_100_complement_after) = table_100();
    assert_eq!(
        (fwmark_rules_after, local_routes_after),
        (1, 1),
        "exactly one owned rule and route after repair"
    );
    assert_eq!(rpdb_complement_after, rpdb_complement_before, "the RPDB complement is unchanged");
    assert_eq!(
        table_100_complement_after, table_100_complement_before,
        "the table-100 complement is unchanged"
    );
    if loss != InterceptLoss::ProgramTable {
        assert_eq!(owned_rules(), rules_before, "an intact program is never rewritten");
    }

    let third = member_spec("nd61-third", THIRD_GUEST);
    worker.start_alloc(&third).await.expect(
        "a new allocation installs after repair: the prior node guard was handed over, so the \
         recorded targets survived",
    );
    let with_third = host
        .observe_shared_state()
        .expect("the intercept state is observable")
        .expect("the owned program is present");
    assert_eq!(
        with_third,
        InterceptState {
            members: members_of(&[FIRST_GUEST, SECOND_GUEST, THIRD_GUEST]),
            ..baseline.clone()
        },
        "the new allocation's members join the unchanged program"
    );
    worker.audit_shared_owner().await.expect("the owner audits clean with the new allocation");

    for spec in [&third, &second, &first] {
        worker.stop_alloc(&spec.alloc).await.expect("each live allocation stops cleanly");
    }
    worker.shutdown_owner().await.expect("the owner shuts down cleanly");
    evidence.record(row, "done", "repaired, audited clean, admitted a new allocation, stopped");
}

/// Each intercept object deleted from under live allocations is detected by
/// the worker's audit within a second, attributed to its one component, and
/// restored exactly by `converge_shared_owner` (feature delta *Runtime member
/// audit (R15)*, *Runtime repair contract*; E13's Lima column, E11's
/// policy-route case). Rows, each on a freshly scrubbed kernel with two live
/// allocations: one member (`IpSets`), the whole program table, the fwmark
/// rule, and table 100's local route (each `IpRules`). After every repair the
/// port's `observe_shared_state` equals the pre-loss state, the fwmark rule
/// and local route are present exactly once (`Client::local_route_present`),
/// the RPDB and table-100 complements are unchanged, an intact program keeps
/// its rule handles (no rewrite), and a new allocation installs — the prior
/// node guard was handed over, not dropped, so the recorded targets survived.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-61 — Lost members, policy route, or guard are detected within a second
/// and repaired with live workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]
async fn each_deleted_intercept_object_is_restored_exactly_with_live_allocations() {
    assert!(is_root(), "S-ND295-61 Lima evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-61");
    let evidence = Evidence::begin("S-ND295-61");
    let _kernel_lock = KernelStateLock::acquire();
    for loss in [
        InterceptLoss::Member,
        InterceptLoss::ProgramTable,
        InterceptLoss::FwmarkRule,
        InterceptLoss::LocalRoute,
    ] {
        lose_and_repair(evidence, loss).await;
    }
}

/// The D-295-R18 intercept-mark guard table deleted from under live
/// allocations is detected by the audit within a second as an `IpRules`
/// failure and restored exactly by `converge_shared_owner`, without rewriting
/// the intact program; afterwards a new allocation installs (feature delta
/// *Runtime member audit (R15)*, *Runtime repair contract*, R18 guard
/// presence; E13's guard case). R18-conditional: DELIVER step 08-01 removes
/// this body, and the `MarkGuardTable` row it drives, if R18 is withdrawn.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-61 — Lost members, policy route, or guard are detected within a second
/// and repaired with live workloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]
async fn the_intercept_mark_guard_table_is_restored_exactly_with_live_allocations() {
    assert!(is_root(), "S-ND295-61 Lima evidence requires root and CAP_NET_ADMIN");
    record_uname("S-ND295-61-guard");
    let evidence = Evidence::begin("S-ND295-61");
    let _kernel_lock = KernelStateLock::acquire();
    lose_and_repair(evidence, InterceptLoss::MarkGuardTable).await;
}
