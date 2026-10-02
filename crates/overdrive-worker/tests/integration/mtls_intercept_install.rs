//! Tier-3 integration evidence for host mTLS interception kernel effects and
//! the B-7 shared listener.
//!
//! Proves selected production adapter operations against REAL kernel side
//! effects on the Lima kernel — no mocks, no synthetic ctx:
//!
//!   AC1 `make_transparent_listener` → a listener whose socket has
//!        `IP_TRANSPARENT` set (proven by `getsockopt(SOL_IP,
//!        IP_TRANSPARENT) == 1` on the real bound fd).
//!   AC2 `install_inbound_tproxy` (the (b)-refined multi-virt model) → a
//!        per-virt TPROXY rule is APPENDED to the SHARED `prerouting` chain;
//!        a second install for a different virt COEXISTS (the second install
//!        does NOT raze the first); dropping ONE guard removes ONLY that
//!        virt's rule by handle, leaving the sibling's rule + the shared
//!        chain/exemption/ip-rule/route intact.
//!   S-ND295-70 `HostMtlsIntercept` accepts redirected outbound and inbound
//!        connections through the shared listener port; each accepted
//!        connection reports its original destination as `local` and its
//!        sender as `peer`.
//!   D3  the F5 `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption is present
//!        in the shared chain AND ordered BEFORE any tproxy rule; a dial with
//!        `SO_MARK = MTLS_LEG_S_DIAL_MARK` is NOT redirected to leg C (the
//!        exemption accepts it, no recursion).
//!
//! The kernel-effect bodies assert at the syscall/kernel boundary
//! (`getsockopt`, `nft -a list chain`, `ip rule`). The S-ND295-70 bodies drive
//! `HostMtlsIntercept::bind_transparent` and `InterceptListener::accept` for
//! the real redirected-connect original-destination evidence.
//!
//! Requires root + `CAP_NET_ADMIN` (IP_TRANSPARENT, nft, ip rule/route):
//! run via `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. A non-root run SKIPs (returns early).
//!
//! Hygiene: the shared routing infra (`ip rule`, `ip route`, nft
//! table/chain/exemption) now PERSISTS by design (node-global converge-on-boot
//! per the (b)-refined model), so each test tolerates pre-existing shared
//! infra at setup and scrubs ALL `overdrive-mtls` nft state + the fwmark
//! rule/route at start AND end via `clean_shared_infra()` so a clean-kernel
//! ground-truth run is reproducible. These tests mutate process-global kernel
//! state (the shared host-netns routing tables); a cross-process
//! `flock(2)` lock (`KernelStateLock`, below) serialises the
//! kernel-touching tests so concurrent installs do not race each other's
//! chain dumps. nextest runs each test in a SEPARATE PROCESS, so an
//! in-process `serial_test` lock cannot serialise node-global kernel
//! state — hence the file lock.
//!
//! # The host listener (GH #295, S-ND295-70)
//!
//! Four bodies at the end of this file prove the `HostMtlsIntercept`
//! listener obligations of the feature delta § *Driven port — intercept
//! listener (DISTILL gap B-7)*: a TPROXY-redirected outbound connection
//! reports the destination the guest dialled as `local` and the guest's source
//! as `peer`; a redirected inbound connection reports the virtual address as
//! `local`; a sock-diag destroy of the listening socket ends a pending accept
//! with an `Accept` failure; and an accepted descriptor is blocking and
//! close-on-exec. Each binds through `HostMtlsIntercept::bind_transparent` and
//! accepts through the `LegListener` bridge (`leg_listener.rs`), so it keeps
//! its oracle on both sides of the DELIVER step that changes
//! `bind_transparent`'s return type. They carry the original-destination
//! evidence of the two helper-only bodies above
//! (`worker_intercept_install_leg_acquire_outbound`,
//! `worker_inbound_tproxy_redirect_recovers_orig_dst`) if that step deletes
//! the helpers.
//!
//! Both redirects are the production shared program's. The inbound body
//! registers a destination member; the outbound body composes the outbound
//! divert on a hand-built veth through [`SharedOutboundDivert`]
//! (`converge_shared`, `install_outbound`, and the guest TCX ingress
//! classifier whose intercept mark shared prerouting rule 1 requires). That
//! fixture is shared with `egress_tproxy_capture.rs` and
//! `name_resolve_enforce_consistency.rs`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::match_wildcard_for_single_variants,
    reason = "Test bodies; skip messages go to stderr; failures must panic with informative messages; size_of/AF_INET casts are FFI-width on compile-time constants; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit"
)]

use std::collections::BTreeSet;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use overdrive_core::dataplane::{GUEST_BRIDGE_MAC, MTLS_LEG_S_DIAL_MARK};
use overdrive_dataplane::DEFAULT_PIN_DIR;
use overdrive_dataplane::guest_tcx::{
    GuestTcxCounter, GuestTcxEndpoint, GuestTcxInventoryIdentity, GuestTcxProgram, TcxAttachPoint,
    detach_pinned_link, query_attachment, read_counter,
};
use overdrive_netlink::nft::{self, SharedIpInterceptIdentity, SharedIpInterceptState};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::mtls_intercept::{
    InterceptPostcondition, install_inbound_tproxy, install_outbound_tproxy,
    make_transparent_listener,
};
use overdrive_worker::mtls_intercept_port::{
    HostMtlsIntercept, InterceptAcceptError, InterceptGuard, MtlsIntercept,
};

use super::leg_listener::{LegListener, accept_failure_of, accept_leg_within, spawn_accept_leg};

/// Cross-PROCESS exclusion for the shared host-netns kernel state.
///
/// The `overdrive-mtls` nft table, the `fwmark` ip-rule, and the `table 100`
/// local route are NODE-GLOBAL: every test that installs/asserts on them
/// touches the SAME kernel state. nextest runs each `#[test]` in a SEPARATE
/// PROCESS, so an in-process lock (`serial_test`) does NOT serialise them —
/// two test processes concurrently in `ensure_shared_routing_infra`'s
/// check-then-add window each add the fwmark rule (→ 2, not 1) and interleave
/// chain dumps. An `flock(2)` on a fixed lock file spans processes; the guard
/// holds the exclusive lock for the whole test body and releases on Drop.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    /// Acquire the exclusive cross-process lock (blocking). The lock file is a
    /// fixed well-known path so every test process contends on the same lock.
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the returned fd is
        // adopted by OwnedFd. flock blocks until the exclusive lock is held.
        let fd = unsafe {
            let raw = libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(raw >= 0, "open kernel-state lock file: {}", std::io::Error::last_os_error());
            let rc = libc::flock(raw, libc::LOCK_EX);
            assert!(rc == 0, "flock LOCK_EX: {}", std::io::Error::last_os_error());
            std::os::fd::OwnedFd::from_raw_fd(raw)
        };
        Self { fd }
    }
}

impl Drop for KernelStateLock {
    fn drop(&mut self) {
        // SAFETY: fd is the live lock fd; LOCK_UN releases the advisory lock.
        // (Dropping the fd would release it too, but be explicit.)
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// `IP_TRANSPARENT` sockopt — libc 0.2 does not name it (same as the
/// reference harness `mtls_roles.rs`).
const IP_TRANSPARENT: libc::c_int = 19;

/// The shared fixed fwmark + routing-policy table the (b)-refined model uses.
const TPROXY_FWMARK: u32 = 0x1;
const TPROXY_RT_TABLE: u32 = 100;

/// True iff this process is uid 0 (root). The IP_TRANSPARENT setopt, nft,
/// and `ip rule`/`route` all need root + CAP_NET_ADMIN; a non-root run
/// cannot stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Read `getsockopt(SOL_IP, IP_TRANSPARENT)` on `fd`. Returns the raw int
/// value (1 == set). Panics on syscall failure — the fixture precondition
/// is "the fd is a real bound socket".
fn getsockopt_ip_transparent(fd: i32) -> libc::c_int {
    let mut val: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: fd is a live socket from the production listener; val/len are
    // correctly sized for an int sockopt.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_IP,
            IP_TRANSPARENT,
            std::ptr::from_mut(&mut val).cast(),
            std::ptr::from_mut(&mut len),
        )
    };
    assert!(rc == 0, "getsockopt(IP_TRANSPARENT): {}", std::io::Error::last_os_error());
    val
}

/// `nft -a list chain ip overdrive-mtls prerouting` — Ok(dump) on a present
/// chain, Err(stderr) on absent. `-a` emits the per-rule `# handle <N>` so the
/// test can assert on rule presence/ordering and handles.
fn nft_list_chain() -> Result<String, String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", "overdrive-mtls", "prerouting"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn nft: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// True iff the chain dump contains a per-virt tproxy rule for `virt`.
fn chain_has_virt_rule(dump: &str, virt: SocketAddrV4) -> bool {
    let daddr = format!("ip daddr {}", virt.ip());
    let dport = format!("tcp dport {}", virt.port());
    dump.lines().any(|l| l.contains(&daddr) && l.contains(&dport) && l.contains("tproxy to"))
}

/// True iff an `ip rule` line for `fwmark <mark>` lookup `<table>` exists.
fn ip_rule_fwmark_present(mark: u32, table: u32) -> bool {
    ip_rule_fwmark_count(mark, table) > 0
}

/// Count of `ip rule` lines matching `fwmark <mark>` lookup `<table>`. The
/// (b)-refined `ensure_shared_routing_infra` adds the rule only when missing,
/// so a green run leaves EXACTLY ONE regardless of how many virts install.
fn ip_rule_fwmark_count(mark: u32, table: u32) -> usize {
    let out = Command::new("ip")
        .args(["rule", "show"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return 0 };
    let text = String::from_utf8_lossy(&out.stdout);
    let needle_mark = format!("fwmark {mark:#x}");
    let needle_mark_dec = format!("fwmark {mark}");
    text.lines()
        .filter(|l| {
            (l.contains(&needle_mark) || l.contains(&needle_mark_dec))
                && l.contains(&format!("lookup {table}"))
        })
        .count()
}

/// True iff `ip route show table <table>` carries the shared local catch-all
/// loopback route. The kernel CANONICALISES `local 0.0.0.0/0 dev lo` to
/// `local default dev lo scope host` on read, so the needle must match
/// `default`, not `0.0.0.0/0`.
fn ip_route_local_present(table: u32) -> bool {
    let out = Command::new("ip")
        .args(["route", "show", "table", &table.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return false };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|l| l.contains("local") && l.contains("default") && l.contains("lo"))
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing shared infra) AND END. Best-effort: every command's failure is
/// the "nothing to clean" signal, so non-zero exits are intentionally ignored.
fn clean_shared_infra() {
    // Drain however many fwmark rules a prior run may have stacked (a healthy
    // (b) run leaves exactly one; an old buggy run may have stacked several).
    for _ in 0..64 {
        let ok = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            break;
        }
    }
    let _ = Command::new("ip")
        .args(["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

const D15_FOREIGN_TABLE: &str = "nd295_d15_foreign";
const D15_LEG_F_OLD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 31_501);
const D15_LEG_C_OLD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 31_502);
const D15_LEG_F_NEW: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 31_601);
const D15_LEG_C_NEW: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 31_602);

fn nft_script(script: &str) {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nft fixture transaction");
    child
        .stdin
        .as_mut()
        .expect("nft fixture stdin")
        .write_all(script.as_bytes())
        .expect("write nft fixture transaction");
    let output = child.wait_with_output().expect("wait for nft fixture transaction");
    assert!(
        output.status.success(),
        "nft fixture transaction failed: {}\nscript:\n{script}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn nft_table_json(family: &str, table: &str) -> Option<Vec<u8>> {
    let output = Command::new("nft")
        .args(["-j", "-a", "list", "table", family, table])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn nft JSON table dump");
    output.status.success().then_some(output.stdout)
}

fn d15_target_snapshot() -> Vec<(String, Vec<u8>)> {
    ["ip", "bridge"]
        .into_iter()
        .filter_map(|family| {
            nft_table_json(family, "overdrive-mtls").map(|bytes| (family.to_owned(), bytes))
        })
        .collect()
}

fn create_d15_foreign_sentinel() {
    nft_script(&format!(
        "add table ip {D15_FOREIGN_TABLE}\nadd chain ip {D15_FOREIGN_TABLE} sentinel\nadd rule ip {D15_FOREIGN_TABLE} sentinel counter accept comment \"d15-foreign-sentinel\"\n"
    ));
}

fn clean_d15_shared_ip_fixture() {
    for (family, table) in
        [("ip", "overdrive-mtls"), ("bridge", "overdrive-mtls"), ("ip", D15_FOREIGN_TABLE)]
    {
        let _ = Command::new("nft")
            .args(["delete", "table", family, table])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct D15KernelSandbox;

impl D15KernelSandbox {
    fn fresh() -> Self {
        clean_d15_shared_ip_fixture();
        Self
    }
}

impl Drop for D15KernelSandbox {
    fn drop(&mut self) {
        clean_d15_shared_ip_fixture();
    }
}

type ConstantRuleParts<'a> = (&'a [Vec<u8>], &'a [Vec<u8>], &'a [Vec<u8>], &'a [Vec<u8>]);

#[derive(Clone, Copy, Debug)]
enum D15AmbiguousState {
    ForeignFamily,
    IncompleteProgram,
    DuplicateOwnedRule,
    UnknownUserdata,
    ConflictingSetSchema,
    ForeignTableChild,
    NonEmptyDynamicSet,
}

fn constant_rule_parts(observation: &InterceptPostcondition) -> ConstantRuleParts<'_> {
    let InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output } =
        observation
    else {
        panic!("shared host adapter must expose the constant-rule identity")
    };
    (table_and_chains, sets, prerouting, output)
}

fn expected_shared_identity(leg_f: SocketAddrV4, leg_c: SocketAddrV4) -> InterceptPostcondition {
    let (table_and_chains, sets, prerouting, output) =
        SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
            .expect("non-zero D15 listener targets form one canonical identity")
            .normalized_parts();
    InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output }
}

fn replace_managed_set_with_conflicting_schema() {
    let prerouting = nft::list_rules("overdrive-mtls", "prerouting")
        .expect("capture canonical prerouting rules before fixture mutation");
    let output = nft::list_rules("overdrive-mtls", "output")
        .expect("capture canonical output rules before fixture mutation");
    for rule in &prerouting {
        nft::delete_rule("overdrive-mtls", "prerouting", rule.handle)
            .expect("remove one prerouting rule while replacing its referenced set schema");
    }
    for rule in &output {
        nft::delete_rule("overdrive-mtls", "output", rule.handle)
            .expect("remove one output rule while replacing its referenced set schema");
    }
    nft_script(
        "delete set ip overdrive-mtls managed_guest_ips\n\
         add set ip overdrive-mtls managed_guest_ips { type inet_service; comment \"d15-conflicting-schema\"; }\n",
    );
    for (chain, rules) in [("prerouting", prerouting), ("output", output)] {
        for (index, rule) in rules.into_iter().enumerate() {
            let result = if index == 0 {
                nft::insert_rule("overdrive-mtls", chain, &rule.normalized_program, &rule.userdata)
            } else {
                nft::append_rule("overdrive-mtls", chain, &rule.normalized_program, &rule.userdata)
            };
            result.expect("restore every canonical rule around the one conflicting set schema");
        }
    }
}

fn assert_three_empty_sets_and_eight_rules(observation: &InterceptPostcondition) {
    let (tables, sets, prerouting, output) = constant_rule_parts(observation);
    assert_eq!(tables.len(), 3, "one table plus two base chains");
    assert_eq!(sets.len(), 3, "exact shared-IP set schemas");
    assert_eq!(prerouting.len(), 5, "five canonical prerouting rules");
    assert_eq!(output.len(), 3, "three canonical output rules");
    for set in ["managed_guest_ips", "outbound_sources", "inbound_destinations"] {
        let json = nft_table_json("ip", "overdrive-mtls")
            .expect("shared-IP table remains present while its guard is owned");
        let rendered = String::from_utf8_lossy(&json);
        assert!(rendered.contains(set), "set `{set}` is present: {rendered}");
    }
    let table = nft_table_json("ip", "overdrive-mtls").expect("shared-IP table dump");
    let rendered = String::from_utf8_lossy(&table);
    assert!(
        !rendered.contains("\"elem\"") && !rendered.contains("\"elements\""),
        "all three dynamic sets remain empty: {rendered}"
    );
}

fn assert_only_listener_targets_changed(
    prior: &InterceptPostcondition,
    replacement: &InterceptPostcondition,
) {
    let (prior_tables, prior_sets, prior_prerouting, prior_output) = constant_rule_parts(prior);
    let (next_tables, next_sets, next_prerouting, next_output) = constant_rule_parts(replacement);
    assert_eq!(prior_tables, next_tables);
    assert_eq!(prior_sets, next_sets);
    assert_eq!(prior_output, next_output);
    assert_eq!(prior_prerouting.len(), 5);
    assert_eq!(next_prerouting.len(), 5);
    for index in [0, 2, 4] {
        assert_eq!(prior_prerouting[index], next_prerouting[index]);
    }
    for index in [1, 3] {
        assert_ne!(prior_prerouting[index], next_prerouting[index]);
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn shared_program_absence_create_readback_idempotence_and_guard_drop() {
    assert!(is_root(), "D15 Lima evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = D15KernelSandbox::fresh();
    create_d15_foreign_sentinel();
    let foreign_before =
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel table exists");
    let host = HostMtlsIntercept::new();

    assert_eq!(host.observe_shared().expect("genuine absence is observable"), None);
    let first_guard = host
        .converge_shared(None, D15_LEG_F_OLD, D15_LEG_C_OLD)
        .expect("one atomic create installs the complete shared-IP program");
    let created = host
        .observe_shared()
        .expect("created program read-back succeeds")
        .expect("created program is present");
    assert_three_empty_sets_and_eight_rules(&created);
    assert_eq!(created, expected_shared_identity(D15_LEG_F_OLD, D15_LEG_C_OLD));

    let mut observer =
        nft::NftRuleObserver::subscribe().expect("subscribe before exact-identity reapply");
    let prerouting_before = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("stable prerouting snapshot before exact reapply");
    let output_before = observer
        .snapshot("overdrive-mtls", "output")
        .expect("stable output snapshot before exact reapply");
    let adopted_guard = host
        .converge_shared(Some(&created), D15_LEG_F_OLD, D15_LEG_C_OLD)
        .expect("identical reapply adopts without mutation");
    let prerouting_after = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("stable prerouting snapshot after exact reapply");
    let output_after = observer
        .snapshot("overdrive-mtls", "output")
        .expect("stable output snapshot after exact reapply");
    assert_eq!(prerouting_after, prerouting_before);
    assert_eq!(output_after, output_before);
    observer.ensure_no_notifications().expect("exact reapply emits no nft mutation notification");
    assert_eq!(host.observe_shared().expect("idempotent identity read-back"), Some(created));

    drop(adopted_guard);
    assert_eq!(
        host.observe_shared().expect("unpublished guard cleanup is observable"),
        None,
        "guard Drop conditionally deletes the complete owned object graph"
    );
    drop(first_guard);
    assert_eq!(
        host.observe_shared().expect("second stale guard is harmless"),
        None,
        "stale conditional cleanup cannot recreate or delete foreign state"
    );
    assert_eq!(
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel survives"),
        foreign_before
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn shared_program_replaces_only_listener_targets_and_preserves_foreign_complement() {
    assert!(is_root(), "D15 Lima evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = D15KernelSandbox::fresh();
    create_d15_foreign_sentinel();
    let foreign_before =
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel table exists");
    let host = HostMtlsIntercept::new();

    let prior_guard = host
        .converge_shared(None, D15_LEG_F_OLD, D15_LEG_C_OLD)
        .expect("seed exact owned prior through the public host adapter");
    let prior = host
        .observe_shared()
        .expect("old targets read back")
        .expect("old target identity is present");
    assert_three_empty_sets_and_eight_rules(&prior);
    assert_eq!(prior, expected_shared_identity(D15_LEG_F_OLD, D15_LEG_C_OLD));

    let replacement_guard = host
        .converge_shared(Some(&prior), D15_LEG_F_NEW, D15_LEG_C_NEW)
        .expect("one atomic target-only replacement succeeds");
    let replacement = host
        .observe_shared()
        .expect("new targets read back")
        .expect("new target identity is present");
    assert_three_empty_sets_and_eight_rules(&replacement);
    assert_only_listener_targets_changed(&prior, &replacement);
    assert_eq!(replacement, expected_shared_identity(D15_LEG_F_NEW, D15_LEG_C_NEW));
    assert_eq!(
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel survives replacement"),
        foreign_before
    );

    drop(prior_guard);
    assert_eq!(
        host.observe_shared().expect("stale old guard cannot delete retargeted state"),
        Some(replacement)
    );
    drop(replacement_guard);
    assert_eq!(host.observe_shared().expect("current unpublished guard restores absence"), None);
    assert_eq!(
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel survives cleanup"),
        foreign_before
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn shared_program_valid_wrong_target_observation_is_non_mutating() {
    assert!(is_root(), "D15 Lima evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = D15KernelSandbox::fresh();
    create_d15_foreign_sentinel();
    let foreign_before =
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel table exists");
    let host = HostMtlsIntercept::new();
    let guard = host
        .converge_shared(None, D15_LEG_F_NEW, D15_LEG_C_NEW)
        .expect("seed a canonical different non-zero target through the public host adapter");
    let recorded_identity = expected_shared_identity(D15_LEG_F_OLD, D15_LEG_C_OLD);
    let wrong_target_identity = expected_shared_identity(D15_LEG_F_NEW, D15_LEG_C_NEW);
    assert_ne!(wrong_target_identity, recorded_identity);

    let target_before = d15_target_snapshot();
    let mut observer = nft::NftRuleObserver::subscribe()
        .expect("subscribe before the non-mutating runtime observation");
    let prerouting_before = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("stable prerouting snapshot before runtime observation");
    let output_before = observer
        .snapshot("overdrive-mtls", "output")
        .expect("stable output snapshot before runtime observation");

    assert_eq!(
        host.observe_shared().expect("canonical wrong target is still an owned identity"),
        Some(wrong_target_identity)
    );
    assert_eq!(
        observer
            .snapshot("overdrive-mtls", "prerouting")
            .expect("stable prerouting snapshot after runtime observation"),
        prerouting_before
    );
    assert_eq!(
        observer
            .snapshot("overdrive-mtls", "output")
            .expect("stable output snapshot after runtime observation"),
        output_before
    );
    observer
        .ensure_no_notifications()
        .expect("runtime observation emits no nft mutation notification");
    assert_eq!(d15_target_snapshot(), target_before);
    assert_eq!(
        nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel survives observation"),
        foreign_before
    );

    drop(guard);
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one finite real-kernel table enumerates every accepted ambiguous owned-state partition"
)]
fn shared_program_refuses_ambiguous_owned_state_without_mutation() {
    assert!(is_root(), "D15 Lima evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();

    for row in [
        D15AmbiguousState::ForeignFamily,
        D15AmbiguousState::IncompleteProgram,
        D15AmbiguousState::DuplicateOwnedRule,
        D15AmbiguousState::UnknownUserdata,
        D15AmbiguousState::ConflictingSetSchema,
        D15AmbiguousState::ForeignTableChild,
        D15AmbiguousState::NonEmptyDynamicSet,
    ] {
        let _sandbox = D15KernelSandbox::fresh();
        create_d15_foreign_sentinel();
        let host = HostMtlsIntercept::new();
        let seeded_guard = host
            .converge_shared(None, D15_LEG_F_OLD, D15_LEG_C_OLD)
            .expect("seed exact owned program through HostMtlsIntercept");

        match row {
            D15AmbiguousState::ForeignFamily => {
                nft_script("add table bridge overdrive-mtls\n");
            }
            D15AmbiguousState::IncompleteProgram => {
                let rule = nft::list_rules("overdrive-mtls", "prerouting")
                    .expect("list canonical prerouting rules")
                    .pop()
                    .expect("five canonical prerouting rules");
                nft::delete_rule("overdrive-mtls", "prerouting", rule.handle)
                    .expect("delete one owned rule to seed incompleteness");
            }
            D15AmbiguousState::DuplicateOwnedRule => {
                let rule = nft::list_rules("overdrive-mtls", "prerouting")
                    .expect("list canonical prerouting rules")
                    .into_iter()
                    .next()
                    .expect("five canonical prerouting rules");
                nft::append_rule(
                    "overdrive-mtls",
                    "prerouting",
                    &rule.normalized_program,
                    &rule.userdata,
                )
                .expect("append an exact duplicate owned identity");
            }
            D15AmbiguousState::UnknownUserdata => {
                let rule = nft::list_rules("overdrive-mtls", "prerouting")
                    .expect("list canonical prerouting rules")
                    .into_iter()
                    .next()
                    .expect("five canonical prerouting rules");
                nft::delete_rule("overdrive-mtls", "prerouting", rule.handle)
                    .expect("delete one canonical rule before changing userdata");
                nft::insert_rule(
                    "overdrive-mtls",
                    "prerouting",
                    &rule.normalized_program,
                    b"foreign-d15-userdata",
                )
                .expect("restore the canonical program with unknown userdata");
            }
            D15AmbiguousState::ConflictingSetSchema => {
                replace_managed_set_with_conflicting_schema();
            }
            D15AmbiguousState::ForeignTableChild => {
                nft_script("add chain ip overdrive-mtls foreign_child\n");
            }
            D15AmbiguousState::NonEmptyDynamicSet => {
                nft_script("add element ip overdrive-mtls managed_guest_ips { 100.95.0.2 }\n");
            }
        }

        let target_before = d15_target_snapshot();
        let foreign_before =
            nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel before refusal");
        assert!(
            host.observe_shared().is_err(),
            "{row:?}: non-repairing observation refuses ambiguous owned state"
        );
        assert!(
            host.converge_shared(None, D15_LEG_F_NEW, D15_LEG_C_NEW).is_err(),
            "{row:?}: convergence refuses before target mutation"
        );
        assert_eq!(d15_target_snapshot(), target_before, "{row:?}: target state is byte-equal");
        assert_eq!(
            nft_table_json("ip", D15_FOREIGN_TABLE).expect("foreign sentinel after refusal"),
            foreign_before,
            "{row:?}: unrelated foreign table is byte-equal"
        );

        drop(seeded_guard);
        assert_eq!(
            d15_target_snapshot(),
            target_before,
            "{row:?}: a stale unpublished guard cannot delete changed or ambiguous state"
        );
    }
}

const ADOPTION_NS: &str = "ns-d7-adopt";
const ADOPTION_GUEST_VETH: &str = "d7-adopt-wl";
const ADOPTION_HOST_VETH: &str = "d7-adopt-veth";

fn adoption_ip(args: &[&str]) {
    let output = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip for the D7 adoption topology");
    assert!(
        output.status.success(),
        "ip {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn adoption_ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

struct AdoptionTopology;

impl AdoptionTopology {
    fn provision() -> Self {
        Self::teardown();
        adoption_ip(&["netns", "add", ADOPTION_NS]);
        adoption_ip(&[
            "link",
            "add",
            ADOPTION_GUEST_VETH,
            "type",
            "veth",
            "peer",
            "name",
            ADOPTION_HOST_VETH,
        ]);
        adoption_ip(&["link", "set", ADOPTION_GUEST_VETH, "netns", ADOPTION_NS]);
        adoption_ip(&["addr", "add", "10.253.71.1/24", "dev", ADOPTION_HOST_VETH]);
        adoption_ip(&["link", "set", ADOPTION_HOST_VETH, "up"]);
        adoption_ip(&["netns", "exec", ADOPTION_NS, "ip", "link", "set", "lo", "up"]);
        adoption_ip(&[
            "netns",
            "exec",
            ADOPTION_NS,
            "ip",
            "addr",
            "add",
            "10.253.71.2/24",
            "dev",
            ADOPTION_GUEST_VETH,
        ]);
        adoption_ip(&[
            "netns",
            "exec",
            ADOPTION_NS,
            "ip",
            "link",
            "set",
            ADOPTION_GUEST_VETH,
            "up",
        ]);
        adoption_ip(&[
            "netns",
            "exec",
            ADOPTION_NS,
            "ip",
            "route",
            "add",
            "default",
            "via",
            "10.253.71.1",
        ]);
        let _ = Command::new("sysctl")
            .args(["-w", &format!("net.ipv4.conf.{ADOPTION_HOST_VETH}.rp_filter=0")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Self
    }

    fn teardown() {
        adoption_ip_quiet(&["link", "del", ADOPTION_HOST_VETH]);
        adoption_ip_quiet(&["netns", "del", ADOPTION_NS]);
    }

    fn drive_one_tcp_syn() {
        let script = "import socket\ns=socket.socket()\ns.settimeout(1)\ns.connect_ex(('198.18.0.1',24443))\ns.close()";
        let output = Command::new("ip")
            .args(["netns", "exec", ADOPTION_NS, "python3", "-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("drive one D7-counted TCP SYN from the workload netns");
        assert!(
            output.status.success(),
            "workload TCP probe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
}

impl Drop for AdoptionTopology {
    fn drop(&mut self) {
        Self::teardown();
    }
}

fn exact_tagged_rule(tag: &[u8]) -> overdrive_netlink::nft::RuleInfo {
    let matching = overdrive_netlink::nft::list_rules("overdrive-mtls", "prerouting")
        .expect("strict GETRULE dump")
        .into_iter()
        .filter(|rule| rule.userdata == tag)
        .collect::<Vec<_>>();
    let [rule] = matching.as_slice() else {
        panic!("expected exactly one production rule for tag {tag:?}, got {matching:#?}");
    };
    rule.clone()
}

fn snapshot_tagged_rule(
    snapshot: &overdrive_netlink::nft::RuleSnapshot,
    tag: &[u8],
) -> overdrive_netlink::nft::RuleInfo {
    let matching = snapshot.rules.iter().filter(|rule| rule.userdata == tag).collect::<Vec<_>>();
    let [rule] = matching.as_slice() else {
        panic!("expected exactly one snapshotted production rule for tag {tag:?}");
    };
    (*rule).clone()
}

/// Dial `addr` once and return the connected stream so the production
/// `accept_*` fn has a peer to accept.
fn dial(addr: SocketAddrV4, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), timeout)?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

/// Dial `addr` with `SO_MARK = mark` set on the socket BEFORE connect (the
/// shape the agent's own leg-S dial uses). Returns the connected stream.
fn dial_with_so_mark(
    addr: SocketAddrV4,
    mark: u32,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    // SAFETY: a fresh AF_INET stream socket; SO_MARK is set on it before
    // connect; the fd is adopted by TcpStream::from_raw_fd which owns it.
    let stream = unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mark_val: libc::c_int = mark as libc::c_int;
        let rc = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark_val).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        TcpStream::from_raw_fd(fd)
    };
    let sa = std::net::SocketAddr::V4(addr);
    // std has no connect_timeout-on-existing-fd; the loopback connect is
    // immediate. Set a short read timeout so a hung connect cannot stall.
    stream.connect_timeout_compat(&sa, timeout)?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

/// `TcpStream` does not expose connect on an existing fd; emulate it for the
/// SO_MARK case via a raw `connect(2)` with a bounded poll.
trait ConnectCompat {
    fn connect_timeout_compat(
        &self,
        addr: &std::net::SocketAddr,
        timeout: Duration,
    ) -> std::io::Result<()>;
}

impl ConnectCompat for TcpStream {
    fn connect_timeout_compat(
        &self,
        addr: &std::net::SocketAddr,
        timeout: Duration,
    ) -> std::io::Result<()> {
        let std::net::SocketAddr::V4(v4) = addr else {
            return Err(std::io::Error::other("v4 only"));
        };
        let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = v4.port().to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
        self.set_read_timeout(Some(timeout)).ok();
        // SAFETY: self owns a live AF_INET socket fd; sa is a correctly-sized
        // sockaddr_in for the connect target.
        let rc = unsafe {
            libc::connect(
                self.as_raw_fd(),
                std::ptr::from_ref(&sa).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// AC1: `make_transparent_listener` sets IP_TRANSPARENT on the real socket.
#[test]
fn worker_make_transparent_listener_sets_ip_transparent() {
    if !is_root() {
        eprintln!("SKIP worker_make_transparent_listener_sets_ip_transparent: not root");
        return;
    }
    let listener = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener must bind an IP_TRANSPARENT socket");
    let val = getsockopt_ip_transparent(listener.as_raw_fd());
    assert_eq!(val, 1, "IP_TRANSPARENT must be set on the production leg-C listener");
    // And it is actually bound to a loopback addr with an assigned port.
    let addr = listener.local_addr().expect("local_addr");
    assert!(addr.is_ipv4(), "must be a v4 loopback listener");
    assert_ne!(addr.port(), 0, "kernel must have assigned a port");
}

/// AC2 (multi-virt coexistence + per-virt by-handle teardown) + D3 (F5
/// exemption present and ordered first).
///
/// The D2-proof: install for virt A, then virt B; assert BOTH per-virt rules
/// coexist in the ONE shared chain (the second install did NOT raze the
/// first). Drop A's guard; assert A's rule is gone AND B's rule + the shared
/// chain/exemption/ip-rule/route all REMAIN. This is the honest AC the old
/// single-table razes-the-table model could never pass.
#[test]
fn worker_inbound_multi_virt_coexist_and_per_virt_teardown() {
    if !is_root() {
        eprintln!("SKIP worker_inbound_multi_virt_coexist_and_per_virt_teardown: not root");
        return;
    }
    // Cross-process exclusion: hold the shared-kernel-state lock for the whole
    // body so a sibling test process cannot mutate the nft chain / fwmark rule
    // concurrently.
    let _kernel_lock = KernelStateLock::acquire();
    // Tolerate pre-existing shared infra; start from a clean kernel.
    clean_shared_infra();

    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    let virt_a = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 5), 18555);
    let virt_b = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 6), 18666);

    let guard_a = install_inbound_tproxy(virt_a, agent_port)
        .expect("install_inbound_tproxy(virt_a) must append a per-virt rule to the shared chain");

    // Second install for a DIFFERENT virt must NOT raze the first.
    let guard_b = install_inbound_tproxy(virt_b, agent_port)
        .expect("install_inbound_tproxy(virt_b) must coexist with virt_a's rule");

    // AC2 (coexistence): BOTH per-virt rules present in the ONE shared chain.
    let dump = nft_list_chain().expect("shared overdrive-mtls prerouting chain must be present");
    assert!(
        chain_has_virt_rule(&dump, virt_a),
        "virt_a's tproxy rule must survive virt_b's install (no raze), got:\n{dump}"
    );
    assert!(
        chain_has_virt_rule(&dump, virt_b),
        "virt_b's tproxy rule must coexist with virt_a's, got:\n{dump}"
    );
    assert!(
        dump.contains(&format!("tproxy to 127.0.0.1:{agent_port}")),
        "both rules redirect to the agent leg-C port, got:\n{dump}"
    );

    // D3: the F5 leg-S-dial exemption is present AND ordered BEFORE any tproxy
    // rule (so the agent's own marked dial is accepted before a redirect can
    // match it).
    // nft renders the mark zero-padded 8-hex (e.g. `meta mark 0x00000002
    // accept`), NOT `0x2` / decimal `2`. Match the canonical rendering.
    let exemption_needle = format!("meta mark {MTLS_LEG_S_DIAL_MARK:#010x} accept");
    let exemption_idx = dump
        .lines()
        .position(|l| l.contains(&exemption_needle))
        .unwrap_or_else(|| panic!("F5 leg-S exemption missing from chain, got:\n{dump}"));
    let first_tproxy_idx = dump
        .lines()
        .position(|l| l.contains("tproxy to"))
        .unwrap_or_else(|| panic!("expected at least one tproxy rule, got:\n{dump}"));
    assert!(
        exemption_idx < first_tproxy_idx,
        "F5 exemption (line {exemption_idx}) must precede every tproxy rule (first at {first_tproxy_idx}), got:\n{dump}"
    );

    // Shared infra present.
    assert_eq!(
        ip_rule_fwmark_count(TPROXY_FWMARK, TPROXY_RT_TABLE),
        1,
        "idempotent ensure leaves EXACTLY ONE shared fwmark rule across two installs"
    );
    assert!(
        ip_route_local_present(TPROXY_RT_TABLE),
        "shared local route in table 100 must be present"
    );

    // AC2 (per-virt teardown): drop A only.
    drop(guard_a);
    let dump_after =
        nft_list_chain().expect("shared chain must STILL be present after dropping one guard");
    assert!(
        !chain_has_virt_rule(&dump_after, virt_a),
        "dropping guard_a must remove ONLY virt_a's rule, got:\n{dump_after}"
    );
    // The sibling's rule + the shared infra (chain, exemption, ip-rule, route)
    // must all REMAIN — the by-handle delete touched only virt_a.
    assert!(
        chain_has_virt_rule(&dump_after, virt_b),
        "virt_b's rule must survive virt_a's guard drop, got:\n{dump_after}"
    );
    assert!(
        dump_after.lines().any(|l| l.contains(&exemption_needle)),
        "F5 exemption must survive a per-virt guard drop, got:\n{dump_after}"
    );
    assert!(
        ip_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE),
        "shared fwmark rule must survive a per-virt guard drop"
    );
    assert!(
        ip_route_local_present(TPROXY_RT_TABLE),
        "shared local route must survive a per-virt guard drop"
    );

    drop(guard_b);
    clean_shared_infra();
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn shared_tproxy_infrastructure_converges_on_second_ensure() {
    if !is_root() {
        eprintln!("SKIP shared_tproxy_infrastructure_converges_on_second_ensure: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let first = install_outbound_tproxy("d7-converge-a", 31_001)
        .expect("first install creates the shared infrastructure");
    let second = install_outbound_tproxy("d7-converge-b", 31_002)
        .expect("second install adopts the shared infrastructure");
    let dump = nft_list_chain().expect("shared prerouting chain");
    let exemptions = dump
        .lines()
        .filter(|line| line.contains(&format!("meta mark {MTLS_LEG_S_DIAL_MARK:#010x} accept")))
        .count();

    assert_eq!(exemptions, 1, "the second ensure preserves one shared exemption: {dump}");
    assert_eq!(
        ip_rule_fwmark_count(TPROXY_FWMARK, TPROXY_RT_TABLE),
        1,
        "the second ensure preserves one shared fwmark rule"
    );
    assert!(ip_route_local_present(TPROXY_RT_TABLE));

    drop(second);
    drop(first);
    clean_shared_infra();
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn same_egress_guard_install_twice_adopts_one_rule() {
    if !is_root() {
        eprintln!("SKIP same_egress_guard_install_twice_adopts_one_rule: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let _topology = AdoptionTopology::provision();
    let host_veth = ADOPTION_HOST_VETH;
    let agent_port = 31_111;
    let first = install_outbound_tproxy(host_veth, agent_port)
        .expect("first install appends the production egress rule");
    let tag = overdrive_netlink::nft::userdata_egress(host_veth, agent_port);
    AdoptionTopology::drive_one_tcp_syn();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let accumulated = loop {
        let rule = exact_tagged_rule(&tag);
        if rule.counter.is_some_and(|counter| counter.packets > 0 && counter.bytes > 0) {
            break rule;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the production-owned D7 rule must accumulate a non-zero counter"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        accumulated.normalized_program,
        overdrive_netlink::nft::normalized_rule_program_identity(
            &overdrive_netlink::nft::egress_tproxy_rule_exprs(
                host_veth,
                Ipv4Addr::LOCALHOST,
                agent_port,
                0x1,
            ),
        )
        .expect("production program normalizes"),
        "the kernel projection is byte-exact with the normalized production encoder"
    );
    let mut observer = overdrive_netlink::nft::NftRuleObserver::subscribe()
        .expect("subscribe strict read-only observer");
    let before_adoption = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("strict GETGEN/GETRULE/GETGEN snapshot");
    assert!(before_adoption.generation != 0, "strict GETGEN returns the full non-zero generation");
    let before_rule = snapshot_tagged_rule(&before_adoption, &tag);
    assert_eq!(before_rule, accumulated, "the stable pre-adoption snapshot is exact");
    observer.ensure_no_notifications().expect("no mutation notification after the stable snapshot");

    let second = install_outbound_tproxy(host_veth, agent_port)
        .expect("second install adopts the exact existing rule");
    let after_adoption = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("adoption leaves one strict stable snapshot");
    assert_eq!(
        after_adoption.generation, before_adoption.generation,
        "same-tag adoption does not mutate the ruleset generation"
    );
    assert_eq!(
        snapshot_tagged_rule(&after_adoption, &tag),
        before_rule,
        "same-tag adoption preserves handle/userdata/program/non-zero counter byte-exact"
    );
    observer.ensure_no_notifications().expect("same-tag adoption emits no nft mutation");

    drop(first);
    let after_first_drop = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("the first shared-owner drop leaves the exact rule live");
    assert_eq!(after_first_drop.generation, before_adoption.generation);
    assert_eq!(snapshot_tagged_rule(&after_first_drop, &tag), before_rule);
    observer
        .ensure_no_notifications()
        .expect("dropping one of two guards cannot delete or replace the shared rule");

    drop(second);
    assert!(
        overdrive_netlink::nft::list_rules("overdrive-mtls", "prerouting")
            .expect("strict post-final-drop GETRULE dump")
            .iter()
            .all(|rule| rule.userdata != tag),
        "the final guard tears down the production rule"
    );
    assert!(
        observer.ensure_no_notifications().is_err(),
        "the final shared-owner drop emits the one expected deletion notification"
    );
    let mut post_delete_observer = overdrive_netlink::nft::NftRuleObserver::subscribe()
        .expect("subscribe after the final deletion");
    let post_delete = post_delete_observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("strict snapshot after the final deletion");
    assert_eq!(
        post_delete.generation,
        before_adoption.generation.wrapping_add(1),
        "exactly one nft mutation occurs across both guard drops"
    );
    assert!(post_delete.rules.iter().all(|rule| rule.userdata != tag));
    post_delete_observer
        .ensure_no_notifications()
        .expect("post-delete observer sees no second teardown");
    clean_shared_infra();
}

/// D3 (recursion bypass): a dial carrying `SO_MARK = MTLS_LEG_S_DIAL_MARK`
/// (the shape the agent's own leg-S dial uses) is NOT TPROXY-redirected onto
/// leg C — the F5 exemption at the chain head accepts it first. We prove this
/// by binding a REAL server on `virt` and asserting the marked dial reaches
/// THAT server (not leg C): if the redirect had fired, the marked connection
/// would have landed on leg C and the real server would never accept it.
#[test]
fn worker_inbound_leg_s_marked_dial_bypasses_redirect() {
    if !is_root() {
        eprintln!("SKIP worker_inbound_leg_s_marked_dial_bypasses_redirect: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    // leg-C listener: the redirect TARGET. If the exemption is broken, the
    // marked dial would be redirected here.
    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    // A REAL server bound on the virt addr — the leg-S-marked dial must reach
    // THIS, proving it bypassed the redirect. Use a concrete loopback addr +
    // port we can bind directly.
    let virt = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 7), 18777);
    let real_server = std::net::TcpListener::bind(virt).expect("bind real server on virt");

    let guard = install_inbound_tproxy(virt, agent_port)
        .expect("install_inbound_tproxy must append the per-virt TPROXY rule");

    // The marked dial: SO_MARK = MTLS_LEG_S_DIAL_MARK. The F5 exemption
    // (ordered first) accepts it in prerouting before the tproxy rule matches,
    // so it is delivered to the REAL server on virt, not redirected to leg C.
    let client = std::thread::spawn(move || {
        let s = dial_with_so_mark(virt, MTLS_LEG_S_DIAL_MARK, Duration::from_secs(8));
        if let Ok(mut s) = s {
            let _ = s.write_all(b"MARKED");
            std::thread::sleep(Duration::from_millis(200));
        }
    });

    // The real server accepts (proving bypass). leg C is NOT polled — if the
    // redirect had wrongly fired, the marked dial would have landed on leg C
    // and this accept would time out.
    real_server.set_nonblocking(false).expect("blocking accept on real server");
    let (mut conn, _peer) = accept_with_timeout(&real_server, Duration::from_secs(5))
        .expect("F5 exemption: leg-S-marked dial must reach the REAL server on virt, not leg C");
    let mut buf = [0u8; 6];
    conn.read_exact(&mut buf).expect("read the marked-dial payload");
    assert_eq!(&buf, b"MARKED", "the real server must receive the marked dial's bytes");

    client.join().expect("marked-dial client thread");
    // leg_c is held only so the redirect target exists; never accepted from.
    drop(leg_c);
    drop(guard);
    clean_shared_infra();
}

/// Accept on `listener` within `timeout` by polling with a short read-timeout
/// loop on a non-blocking accept. Returns the accepted connection or an error
/// if nothing arrives within the budget (the failure shape that would mean the
/// marked dial was wrongly redirected to leg C).
fn accept_with_timeout(
    listener: &std::net::TcpListener,
    timeout: Duration,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false).ok();
                return Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no inbound connection within timeout (marked dial may have been redirected)",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

// ===========================================================================
// Node-global sysctls of a hand-built veth topology (shared fixture)
// ===========================================================================

/// The `/proc/sys` path of the dotted sysctl `key`.
fn sysctl_path(key: &str) -> std::path::PathBuf {
    std::path::Path::new("/proc/sys").join(key.replace('.', "/"))
}

/// Read the sysctl `key`, trimmed.
fn read_sysctl(key: &str) -> std::io::Result<String> {
    std::fs::read_to_string(sysctl_path(key)).map(|value| value.trim().to_owned())
}

/// Write `value` to the sysctl `key`, then read it back: a write the kernel
/// accepted but did not apply is an error too.
fn write_sysctl(key: &str, value: &str) -> std::io::Result<()> {
    std::fs::write(sysctl_path(key), value)?;
    let applied = read_sysctl(key)?;
    if applied == value {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("{key} reads back {applied:?} after writing {value:?}")))
    }
}

/// Set a per-link sysctl of a link the fixture created (for example
/// `net.ipv4.conf.<veth>.rp_filter`). The link, and the setting with it, goes
/// when the fixture deletes the link, so nothing needs restoring; the write and
/// its read-back must succeed.
pub(super) fn set_link_sysctl(scenario: &str, key: &str, value: &str) {
    write_sysctl(key, value).unwrap_or_else(|error| {
        panic!("[{scenario}] setting the fixture link's sysctl {key}={value} failed: {error}")
    });
    eprintln!("[{scenario}][sysctl] {key} = {value} (fixture link; removed with the link)");
}

/// Node-global sysctls a hand-built veth topology relaxes (forwarding and
/// reverse-path filtering), snapshotted before the first write and restored on
/// `Drop` — also when the body panics — so the fixture leaves no node-global
/// routing policy behind for a later test or workspace. Every read, write, and
/// read-back is asserted, and every change and restoration is one appended
/// evidence line.
///
/// Consumers: the host-listener topology below, `egress_tproxy_capture.rs`, and
/// `name_resolve_enforce_consistency.rs`.
pub(super) struct NodeSysctls {
    scenario: &'static str,
    /// Each changed key with the value it held before this guard wrote it, in
    /// write order; restored in reverse.
    saved: Vec<(&'static str, String)>,
}

impl NodeSysctls {
    /// Snapshot and set each `(key, value)` in order. A key is recorded for
    /// restoration before it is written, so a failed write still restores the
    /// keys written before it.
    pub(super) fn set(scenario: &'static str, settings: &[(&'static str, &str)]) -> Self {
        let mut guard = Self { scenario, saved: Vec::with_capacity(settings.len()) };
        for (key, value) in settings {
            let prior = read_sysctl(key).unwrap_or_else(|error| {
                panic!("[{scenario}] reading the node-global sysctl {key} failed: {error}")
            });
            guard.saved.push((key, prior.clone()));
            write_sysctl(key, value).unwrap_or_else(|error| {
                panic!("[{scenario}] setting the node-global sysctl {key}={value} failed: {error}")
            });
            eprintln!("[{scenario}][sysctl] {key}: {prior} -> {value} (restored on drop)");
        }
        guard
    }
}

impl Drop for NodeSysctls {
    fn drop(&mut self) {
        let scenario = self.scenario;
        for (key, prior) in self.saved.iter().rev() {
            match write_sysctl(key, prior) {
                Ok(()) => eprintln!("[{scenario}][sysctl] {key} restored to {prior}"),
                // Already unwinding from the body's failure: record the
                // restoration failure beside it rather than aborting.
                Err(error) if std::thread::panicking() => eprintln!(
                    "[{scenario}][sysctl] restoring {key} to {prior} failed during unwind: {error}"
                ),
                Err(error) => {
                    panic!(
                        "[{scenario}] restoring the node-global sysctl {key} to {prior} failed: {error}"
                    )
                }
            }
        }
    }
}

// ===========================================================================
// The production outbound divert on a hand-built veth topology (shared fixture)
// ===========================================================================

/// A locally administered unicast MAC derived from a workload address:
/// `02:00` followed by the address's four octets. A hand-built topology gives
/// its workload-side veth this MAC, so the guest classifier's endpoint record
/// names the source MAC of every frame the workload sends.
pub(super) const fn workload_mac(address: Ipv4Addr) -> [u8; 6] {
    let [a, b, c, d] = address.octets();
    [0x02, 0x00, a, b, c, d]
}

/// The `ip link ... address` text form of `mac` (`02:00:0a:fa:00:02`).
pub(super) fn mac_text(mac: [u8; 6]) -> String {
    mac.iter().map(|octet| format!("{octet:02x}")).collect::<Vec<_>>().join(":")
}

/// Remove `path` and everything below it. An absent path is already removed.
fn remove_pin_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// A cleanup failure fails the body. When the body is already unwinding from
/// its own failure, both are kept: the primary panic, and this line on stderr.
fn report_divert_cleanup_failure(scenario: &str, message: &str) {
    if std::thread::panicking() {
        eprintln!("[{scenario}][divert] cleanup failed during unwind: {message}");
    } else {
        panic!("[{scenario}] divert cleanup failed: {message}");
    }
}

/// The ifindex of the host link `interface`; the read must succeed.
fn link_ifindex(scenario: &str, interface: &str) -> u32 {
    let path = format!("/sys/class/net/{interface}/ifindex");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("[{scenario}] reading {path} failed: {error}"));
    text.trim()
        .parse()
        .unwrap_or_else(|error| panic!("[{scenario}] parsing {path} ({text:?}) failed: {error}"))
}

/// The node's guest TCX ingress classifier (`gh295c_endpoint`), loaded from
/// the embedded object and attached first at one host-side veth's TCX ingress,
/// with the workload's endpoint registered for that veth. It runs in the
/// guest-network owner's provision order for a TAP: load, pin both maps, insert
/// and read back the endpoint, attach first-ingress, pin the link, and query
/// that exactly this program is attached.
///
/// Its pins sit under a test-scoped root,
/// `<DEFAULT_PIN_DIR>/test-worker-divert-<interface>-<pid>`, never under the
/// node's production `mtls-endpoints` pins. [`Self::detach`] detaches the
/// pinned link, requires the attach point to be empty, and removes the pin
/// root, asserting each; `Drop` does the same best-effort when the body did not
/// reach [`Self::detach`]. Loading the object uses the production loader, which
/// reuses (or briefly pins and then removes) the node's `SERVICE_MAP` pin under
/// `DEFAULT_PIN_DIR`; the worker integration binary is `host-kernel-shared`.
struct GuestIngressClassifier {
    scenario: &'static str,
    interface: String,
    pin_root: PathBuf,
    link_pin: PathBuf,
    program: Option<GuestTcxProgram>,
    released: bool,
}

impl GuestIngressClassifier {
    fn attach(scenario: &'static str, interface: &str, endpoint: GuestTcxEndpoint) -> Self {
        let pin_root = Path::new(DEFAULT_PIN_DIR)
            .join(format!("test-worker-divert-{interface}-{}", std::process::id()));
        remove_pin_tree(&pin_root).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] removing the stale divert pin root {} failed: {error}",
                pin_root.display()
            )
        });
        let link_pin = pin_root.join(format!("links/{interface}-ingress"));
        // Constructed before the first pin is written, so a failure below
        // still removes the pin root through `Drop`.
        let mut classifier = Self {
            scenario,
            interface: interface.to_owned(),
            pin_root,
            link_pin,
            program: None,
            released: false,
        };
        let endpoint_pin = classifier.pin_root.join("maps/endpoints");
        let counter_pin = classifier.pin_root.join("maps/counters");
        let (identity, disposition) =
            GuestTcxInventoryIdentity::capture(endpoint_pin.clone(), counter_pin.clone())
                .into_parts();
        disposition.unwrap_or_else(|error| {
            panic!("[{scenario}] the classifier's BPF inventory baseline capture failed: {error:?}")
        });
        let mut program = GuestTcxProgram::load(&identity).unwrap_or_else(|error| {
            panic!("[{scenario}] loading the guest TCX classifier failed: {error:?}")
        });
        program.pin_endpoint_map(&endpoint_pin).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] pinning the endpoint map at {} failed: {error:?}",
                endpoint_pin.display()
            )
        });
        program.pin_counter_map(&counter_pin).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] pinning the counter map at {} failed: {error:?}",
                counter_pin.display()
            )
        });
        let ifindex = link_ifindex(scenario, interface);
        program.insert_endpoint(ifindex, endpoint).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] registering the workload endpoint for {interface} failed: {error:?}"
            )
        });
        assert_eq!(
            program.read_endpoint(ifindex).unwrap_or_else(|error| {
                panic!("[{scenario}] reading back the endpoint of {interface} failed: {error:?}")
            }),
            Some(endpoint),
            "[{scenario}] the endpoint map holds exactly the registered workload endpoint for \
             {interface} (ifindex {ifindex})"
        );
        let link = program.attach_first_ingress(interface).unwrap_or_else(|error| {
            panic!("[{scenario}] attaching the classifier first at {interface} ingress failed: {error:?}")
        });
        let program_id = link.program_id();
        link.pin(&classifier.link_pin).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] pinning the ingress link at {} failed: {error:?}",
                classifier.link_pin.display()
            )
        });
        let attached =
            query_attachment(interface, TcxAttachPoint::Ingress).unwrap_or_else(|error| {
                panic!("[{scenario}] querying the TCX ingress of {interface} failed: {error:?}")
            });
        assert_eq!(
            attached.program_ids,
            vec![program_id],
            "[{scenario}] exactly the loaded classifier is attached at {interface}'s TCX ingress"
        );
        eprintln!(
            "[{scenario}][divert] classifier program {program_id} attached first at {interface} \
             ingress (ifindex {ifindex}); endpoint {endpoint:?}; pins under {}",
            classifier.pin_root.display()
        );
        classifier.program = Some(program);
        classifier
    }

    /// The classifier's count of one semantic outcome; the read must succeed.
    fn counter(&self, counter: GuestTcxCounter) -> u64 {
        let scenario = self.scenario;
        read_counter(self.pin_root.join("maps/counters"), counter).unwrap_or_else(|error| {
            panic!("[{scenario}] reading the classifier's {counter:?} counter failed: {error:?}")
        })
    }

    /// Every loaded counter slot, for the evidence of a failed divert.
    fn counter_snapshot(&self) -> String {
        [
            GuestTcxCounter::GatewayHostPass,
            GuestTcxCounter::Intercept,
            GuestTcxCounter::EndpointMapMiss,
            GuestTcxCounter::SourceMacSpoof,
            GuestTcxCounter::SourceIpArpSpoof,
            GuestTcxCounter::DirectBypassDrop,
            GuestTcxCounter::ArpPass,
            GuestTcxCounter::MalformedDrop,
        ]
        .into_iter()
        .map(|counter| format!("{counter:?}={}", self.counter(counter)))
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn detach(mut self) {
        let scenario = self.scenario;
        let interface = self.interface.clone();
        let program = self.program.take();
        detach_pinned_link(&self.link_pin).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] detaching the pinned ingress link {} failed: {error:?}",
                self.link_pin.display()
            )
        });
        let attached =
            query_attachment(&interface, TcxAttachPoint::Ingress).unwrap_or_else(|error| {
                panic!("[{scenario}] querying the TCX ingress of {interface} failed: {error:?}")
            });
        assert!(
            attached.program_ids.is_empty(),
            "[{scenario}] detaching the classifier empties {interface}'s TCX ingress, got {:?}",
            attached.program_ids
        );
        drop(program);
        remove_pin_tree(&self.pin_root).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] removing the divert pin root {} failed: {error}",
                self.pin_root.display()
            )
        });
        self.released = true;
        eprintln!(
            "[{scenario}][divert] classifier detached from {interface}; pin root {} removed",
            self.pin_root.display()
        );
    }
}

impl Drop for GuestIngressClassifier {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let scenario = self.scenario;
        drop(self.program.take());
        match self.link_pin.try_exists() {
            Ok(true) => {
                if let Err(error) = detach_pinned_link(&self.link_pin) {
                    report_divert_cleanup_failure(
                        scenario,
                        &format!(
                            "detaching the pinned ingress link {}: {error:?}",
                            self.link_pin.display()
                        ),
                    );
                }
            }
            Ok(false) => {}
            Err(error) => report_divert_cleanup_failure(
                scenario,
                &format!("probing the ingress link pin {}: {error}", self.link_pin.display()),
            ),
        }
        if let Err(error) = remove_pin_tree(&self.pin_root) {
            report_divert_cleanup_failure(
                scenario,
                &format!("removing the pin root {}: {error}", self.pin_root.display()),
            );
        }
    }
}

/// The complete shared program state; it must be readable and present.
fn shared_program_state(scenario: &str) -> SharedIpInterceptState {
    nft::observe_shared_ip_intercept_state()
        .unwrap_or_else(|error| {
            panic!("[{scenario}] observing the shared program's members failed: {error:?}")
        })
        .unwrap_or_else(|| panic!("[{scenario}] the shared program is absent"))
}

/// The canonical identity of the shared program at the two legs' targets.
fn shared_ip_identity(leg_f: SocketAddrV4, leg_c: SocketAddrV4) -> SharedIpInterceptIdentity {
    SharedIpInterceptIdentity::for_listener_ports(leg_f.port(), leg_c.port())
        .expect("two bound legs report non-zero ports, which form one canonical identity")
}

/// The production outbound divert under the GH #295 shared design, composed on
/// a hand-built veth topology in place of the guest-network owner's TAP
/// provisioning (which lives in the control plane, outside this crate's
/// dependency graph):
///
/// 1. `HostMtlsIntercept::converge_shared` converges the node's constant
///    program at the two bound legs' targets, from an observed absence;
/// 2. `HostMtlsIntercept::install_outbound(source, leg F's port)` admits the
///    workload's source: one managed-guest and one outbound-source element;
/// 3. the guest TCX ingress classifier is attached first at the host-side
///    veth with the workload's endpoint (source address, source MAC, and the
///    node bridge MAC `GUEST_BRIDGE_MAC`) — [`GuestIngressClassifier`].
///
/// Shared prerouting rule 1 then diverts the workload's TCP to leg F: the
/// classifier stamps the intercept mark `0x295a` on every guest TCP frame it
/// passes, and the source is an admitted outbound source. No per-interface rule is
/// installed. Leg F's replies reach the guest because the transparent listener
/// marks its sockets with the leg-S mark, which output rule 0 exempts ahead of
/// the managed-guest drop.
///
/// Every setup step is asserted. [`Self::release`] undoes them in reverse and
/// asserts each: the classifier detached and its pins removed; the source's
/// elements removed with the program unchanged; then the member-free program
/// removed by the node guard's drop (the feature delta's guard ordering: every
/// element guard before the node guard). On an unwinding panic the fields drop
/// in the same order.
pub(super) struct SharedOutboundDivert {
    scenario: &'static str,
    host: HostMtlsIntercept,
    leg_f: SocketAddrV4,
    leg_c: SocketAddrV4,
    // Field order is the unwinding drop order: the classifier, then the
    // source's elements, then the program.
    classifier: Option<GuestIngressClassifier>,
    admission: Option<Box<dyn InterceptGuard>>,
    program: Option<Box<dyn InterceptGuard>>,
}

impl SharedOutboundDivert {
    /// Compose the divert for `source` (whose frames carry `source_mac`)
    /// arriving on the host-side veth `interface`, to leg F at `leg_f`; `leg_c`
    /// is the program's inbound target.
    pub(super) fn install(
        scenario: &'static str,
        host: &HostMtlsIntercept,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
        source: Ipv4Addr,
        source_mac: [u8; 6],
        interface: &str,
    ) -> Self {
        let prior = host.observe_shared().unwrap_or_else(|error| {
            panic!("[{scenario}] observing the shared program before convergence failed: {error:?}")
        });
        assert_eq!(
            prior, None,
            "[{scenario}] the sandbox leaves no shared program, so convergence starts from an \
             observed absence"
        );
        let program = host.converge_shared(prior.as_ref(), leg_f, leg_c).unwrap_or_else(|error| {
            panic!(
                "[{scenario}] converge_shared at leg F {leg_f} and leg C {leg_c} failed: {error:?}"
            )
        });
        assert_eq!(
            host.observe_shared().unwrap_or_else(|error| {
                panic!("[{scenario}] observing the converged shared program failed: {error:?}")
            }),
            Some(expected_shared_identity(leg_f, leg_c)),
            "[{scenario}] converge_shared installs the constant program at the two legs' targets"
        );
        let admission = host.install_outbound(source, leg_f.port()).unwrap_or_else(|error| {
            panic!("[{scenario}] install_outbound({source}, {}) failed: {error:?}", leg_f.port())
        });
        let admitted = shared_program_state(scenario);
        assert_eq!(
            admitted.identity(),
            &shared_ip_identity(leg_f, leg_c),
            "[{scenario}] admitting the source leaves the constant program unchanged"
        );
        assert_eq!(
            admitted.managed_guest_ips(),
            &BTreeSet::from([source]),
            "[{scenario}] install_outbound adds the source as the one managed guest"
        );
        assert_eq!(
            admitted.outbound_sources(),
            &BTreeSet::from([source]),
            "[{scenario}] install_outbound adds the source as the one outbound source"
        );
        assert!(
            admitted.inbound_destinations().is_empty(),
            "[{scenario}] an outbound admission registers no inbound destination: {admitted:?}"
        );
        eprintln!(
            "[{scenario}][divert] shared program converged at leg F {leg_f} / leg C {leg_c}; \
             source {source} admitted"
        );
        let classifier = GuestIngressClassifier::attach(
            scenario,
            interface,
            GuestTcxEndpoint { source_ipv4: source, source_mac, bridge_mac: GUEST_BRIDGE_MAC },
        );
        Self {
            scenario,
            host: host.clone(),
            leg_f,
            leg_c,
            classifier: Some(classifier),
            admission: Some(admission),
            program: Some(program),
        }
    }

    fn classifier(&self) -> &GuestIngressClassifier {
        self.classifier
            .as_ref()
            .unwrap_or_else(|| unreachable!("the classifier is held until release consumes self"))
    }

    /// How many guest TCP frames the classifier has stamped with the intercept
    /// mark so far.
    pub(super) fn intercepted(&self) -> u64 {
        self.classifier().counter(GuestTcxCounter::Intercept)
    }

    /// Every classifier counter, for the evidence of a failed divert.
    pub(super) fn counter_snapshot(&self) -> String {
        self.classifier().counter_snapshot()
    }

    /// Undo the divert in reverse, asserting each step.
    pub(super) fn release(mut self) {
        let scenario = self.scenario;
        if let Some(classifier) = self.classifier.take() {
            classifier.detach();
        }
        drop(self.admission.take());
        let released = shared_program_state(scenario);
        assert_eq!(
            released.identity(),
            &shared_ip_identity(self.leg_f, self.leg_c),
            "[{scenario}] releasing the admission leaves the constant program unchanged"
        );
        assert!(
            released.managed_guest_ips().is_empty()
                && released.outbound_sources().is_empty()
                && released.inbound_destinations().is_empty(),
            "[{scenario}] releasing the admission removes the source's two elements: {released:?}"
        );
        drop(self.program.take());
        assert_eq!(
            self.host.observe_shared().unwrap_or_else(|error| {
                panic!("[{scenario}] observing the shared program after release failed: {error:?}")
            }),
            None,
            "[{scenario}] the node guard's drop removes the member-free program"
        );
        eprintln!(
            "[{scenario}][divert] released: classifier detached, elements removed, program removed"
        );
    }
}

// ===========================================================================
// S-ND295-70 — the host listener's obligations (GH #295, gap B-7)
// ===========================================================================

/// The workload netns of the outbound host-listener topology.
const HOST_LISTENER_NS: &str = "nsW-hlo70";
/// The workload-side veth (moved into [`HOST_LISTENER_NS`]).
const HOST_LISTENER_VETH_W: &str = "vethW-hlo70";
/// The host-side veth: the guest's dial ingresses it, and the guest classifier
/// runs at its TCX ingress.
const HOST_LISTENER_VETH_H: &str = "vethH-hlo70";
/// The topology's `/24` comes from overdrive-testing's `10.250.0.0/16` pool
/// under this stable owner name; the pool is disjoint from production's
/// `10.99.0.0/16`.
const HOST_LISTENER_LEASE: &str = "worker-mtls-intercept-install-host-listener";
/// The destination the guest dials on the outbound path: a host-lo `/32`
/// disjoint from production and from every sibling fixture's backend. Nothing
/// listens on it. A dial the divert missed never completes: the host's reply to
/// the managed guest carries no leg-S mark, so shared output rule 2 drops it.
const HOST_LISTENER_OUTBOUND_DESTINATION: SocketAddrV4 =
    SocketAddrV4::new(Ipv4Addr::new(10, 201, 70, 1), 18_770);
/// The inbound virtual address (a registered destination and declared port).
const HOST_LISTENER_INBOUND_VIRT: SocketAddrV4 =
    SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 70), 18_707);

/// Scrubs the node-global intercept state these bodies create — both
/// `overdrive-mtls` families, the D15 foreign sentinel, the fwmark rule, and
/// the table-100 local route — at construction and again on drop, so a panic
/// mid-body leaves nothing behind.
struct HostListenerSandbox;

impl HostListenerSandbox {
    fn fresh() -> Self {
        clean_d15_shared_ip_fixture();
        clean_shared_infra();
        Self
    }
}

impl Drop for HostListenerSandbox {
    fn drop(&mut self) {
        clean_d15_shared_ip_fixture();
        clean_shared_infra();
    }
}

/// A workload netns joined to the host by a veth pair, with the host `lo`
/// carrying [`HOST_LISTENER_OUTBOUND_DESTINATION`], so the guest's dial to it
/// ingresses [`HOST_LISTENER_VETH_H`] — where the guest classifier runs — and
/// meets PREROUTING (the `egress_tproxy_capture.rs` recipe). The workload veth
/// carries [`workload_mac`] of its address. Torn down on drop, before the CIDR
/// lease is released and the node-global sysctls are restored.
struct HostListenerEgressTopology {
    lease: TestCidrLease,
    /// Restores forwarding and reverse-path filtering after the teardown.
    _node_sysctls: NodeSysctls,
}

impl HostListenerEgressTopology {
    fn provision() -> Self {
        let lease = TestCidrLease::acquire(HOST_LISTENER_LEASE)
            .expect("acquire the host-listener topology CIDR lease");
        Self::teardown();
        let gateway = format!("{}/{}", lease.host_gateway(), lease.prefix_len());
        let workload = format!("{}/{}", lease.workload_addr(), lease.prefix_len());
        let host_gateway = lease.host_gateway().to_string();
        let destination = format!("{}/32", HOST_LISTENER_OUTBOUND_DESTINATION.ip());
        let mac = mac_text(workload_mac(lease.workload_addr()));
        adoption_ip(&["netns", "add", HOST_LISTENER_NS]);
        adoption_ip(&[
            "link",
            "add",
            HOST_LISTENER_VETH_W,
            "type",
            "veth",
            "peer",
            "name",
            HOST_LISTENER_VETH_H,
        ]);
        // The workload side carries the MAC the classifier's endpoint record
        // names, so its frames pass the source-MAC check.
        adoption_ip(&["link", "set", HOST_LISTENER_VETH_W, "address", &mac]);
        adoption_ip(&["link", "set", HOST_LISTENER_VETH_W, "netns", HOST_LISTENER_NS]);
        adoption_ip(&["addr", "add", &gateway, "dev", HOST_LISTENER_VETH_H]);
        adoption_ip(&["link", "set", HOST_LISTENER_VETH_H, "up"]);
        adoption_ip(&["netns", "exec", HOST_LISTENER_NS, "ip", "link", "set", "lo", "up"]);
        adoption_ip(&[
            "netns",
            "exec",
            HOST_LISTENER_NS,
            "ip",
            "addr",
            "add",
            &workload,
            "dev",
            HOST_LISTENER_VETH_W,
        ]);
        adoption_ip(&[
            "netns",
            "exec",
            HOST_LISTENER_NS,
            "ip",
            "link",
            "set",
            HOST_LISTENER_VETH_W,
            "up",
        ]);
        adoption_ip(&[
            "netns",
            "exec",
            HOST_LISTENER_NS,
            "ip",
            "route",
            "add",
            "default",
            "via",
            &host_gateway,
        ]);
        adoption_ip(&["addr", "add", &destination, "dev", "lo"]);
        // Host-side routing hygiene, as `egress_tproxy_capture.rs` sets it:
        // relaxed reverse-path filtering so the diverted ingress is not dropped.
        // The node-global keys are restored when the topology drops; the host
        // veth's own key goes with the veth. No TX-checksum-offload change is
        // made: the divert rewrites no header, so no checksum base is needed
        // (`.claude/rules/bpf.md` Rule 2 concerns an incremental checksum after
        // a rewrite).
        let node_sysctls = NodeSysctls::set(
            "S-ND295-70",
            &[
                ("net.ipv4.ip_forward", "1"),
                ("net.ipv4.conf.all.rp_filter", "0"),
                ("net.ipv4.conf.lo.rp_filter", "0"),
            ],
        );
        set_link_sysctl(
            "S-ND295-70",
            &format!("net.ipv4.conf.{HOST_LISTENER_VETH_H}.rp_filter"),
            "0",
        );
        Self { lease, _node_sysctls: node_sysctls }
    }

    const fn workload_addr(&self) -> Ipv4Addr {
        self.lease.workload_addr()
    }

    fn teardown() {
        adoption_ip_quiet(&["link", "del", HOST_LISTENER_VETH_H]);
        adoption_ip_quiet(&["netns", "del", HOST_LISTENER_NS]);
        adoption_ip_quiet(&[
            "addr",
            "del",
            &format!("{}/32", HOST_LISTENER_OUTBOUND_DESTINATION.ip()),
            "dev",
            "lo",
        ]);
    }

    /// Dial `destination` from the workload netns, report the client's own
    /// source port as `WL-SRC <port>`, send a marker, and hold the connection
    /// briefly. Returns the client's exit status and output for the evidence.
    fn dial_from_workload(destination: SocketAddrV4) -> String {
        let script = [
            "import socket, time".to_owned(),
            "s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)".to_owned(),
            "s.settimeout(5)".to_owned(),
            "try:".to_owned(),
            format!("    s.connect(('{}',{}))", destination.ip(), destination.port()),
            "    print('WL-SRC %d' % s.getsockname()[1], flush=True)".to_owned(),
            "    s.sendall(b'HOST-LISTENER-OUTBOUND')".to_owned(),
            "    time.sleep(0.5)".to_owned(),
            "except Exception as e:".to_owned(),
            "    print('CLIENT-FAIL:'+str(e))".to_owned(),
        ]
        .join("\n");
        match Command::new("ip")
            .args(["netns", "exec", HOST_LISTENER_NS, "python3", "-c", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) => format!(
                "[exit={:?}] stdout={} stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => format!("spawning the workload client failed: {error}"),
        }
    }
}

impl Drop for HostListenerEgressTopology {
    fn drop(&mut self) {
        Self::teardown();
    }
}

/// The source port a [`HostListenerEgressTopology::dial_from_workload`] client
/// reported, if it connected.
fn reported_workload_source_port(client_output: &str) -> Option<u16> {
    client_output
        .split_whitespace()
        .skip_while(|token| !token.ends_with("WL-SRC"))
        .nth(1)
        .and_then(|port| port.parse().ok())
}

/// The host obligation: for a TPROXY-redirected OUTBOUND connection the
/// accepted `local` is the destination the guest dialled and `peer` is the
/// guest's own source (feature delta § *Driven port — intercept listener*,
/// `HostMtlsIntercept` obligations; D-TME-4). The workload dials a host-side
/// destination through its gateway, and the leg is accepted through the
/// `LegListener` bridge. The client reports its source port, so `peer` is
/// checked exactly.
///
/// The divert is the production shared program, composed on the host veth by
/// [`SharedOutboundDivert`]: `converge_shared` at the two legs bound through
/// `HostMtlsIntercept::bind_transparent`, `install_outbound(workload, leg F's
/// port)`, and the guest TCX classifier at the host veth's ingress, whose
/// intercept mark shared prerouting rule 1 requires. No per-interface rule is
/// installed. The classifier's intercept count must advance with the dial, so
/// the accepted connection is known to have come through that mark.
///
/// Mutation targets: a listener that reports its own bound address as `local`
/// (the orig-dst lost), and one that reports the listener side or a rewritten
/// address as `peer` (the capability source lost).
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_host_listener_reports_a_redirected_outbound_original_destination_as_local() {
    assert!(is_root(), "S-ND295-70 host-listener evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = HostListenerSandbox::fresh();
    let topology = HostListenerEgressTopology::provision();
    let workload = topology.workload_addr();

    let host = HostMtlsIntercept::new();
    let leg_f = Arc::new(
        host.bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("HostMtlsIntercept::bind_transparent binds leg F"),
    );
    let leg_c = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg C");
    let outbound_target = leg_f.bound_v4().expect("leg F reports its bound IPv4 address");
    let inbound_target = leg_c.bound_v4().expect("leg C reports its bound IPv4 address");
    let divert = SharedOutboundDivert::install(
        "S-ND295-70",
        &host,
        outbound_target,
        inbound_target,
        workload,
        workload_mac(workload),
        HOST_LISTENER_VETH_H,
    );
    let intercepted_before = divert.intercepted();
    eprintln!(
        "[S-ND295-70][outbound] leg F {outbound_target}; workload {workload}; destination \
         {HOST_LISTENER_OUTBOUND_DESTINATION}; intercepted before the dial {intercepted_before}"
    );

    let client = std::thread::spawn(|| {
        HostListenerEgressTopology::dial_from_workload(HOST_LISTENER_OUTBOUND_DESTINATION)
    });
    let accepted = accept_leg_within(&leg_f, Duration::from_secs(8));
    let client_output = client.join().expect("the workload client thread completes");
    eprintln!("[S-ND295-70][outbound] workload client: {client_output}");
    let (stream, peer, local) = accepted.unwrap_or_else(|error| {
        panic!(
            "the redirected outbound dial must be accepted on leg F within 8 s; a timeout means \
             the shared program did not divert it ({error}); classifier counters: {}",
            divert.counter_snapshot()
        )
    });
    let intercepted_after = divert.intercepted();
    eprintln!(
        "[S-ND295-70][outbound] accepted peer={peer} local={local}; intercepted after the dial \
         {intercepted_after}"
    );

    assert_eq!(
        local, HOST_LISTENER_OUTBOUND_DESTINATION,
        "the accepted connection's local address is the destination the guest dialled, not leg \
         F's bound address"
    );
    let source_port = reported_workload_source_port(&client_output).unwrap_or_else(|| {
        panic!("the workload client must report its source port: {client_output}")
    });
    assert_eq!(
        peer,
        SocketAddrV4::new(workload, source_port),
        "the accepted peer is the guest's own source address and port"
    );
    assert!(
        intercepted_after > intercepted_before,
        "the guest's TCP reached leg F through the classifier's intercept mark: the Intercept \
         count must advance past {intercepted_before}, got {intercepted_after}"
    );

    drop(stream);
    divert.release();
    drop((leg_c, leg_f));
    drop(topology);
}

/// The host obligation for the INBOUND leg: a connection the owned shared
/// program redirects to leg C reports the virtual address it was sent to as
/// `local`, and its sender as `peer` (feature delta § *Driven port — intercept
/// listener*, `HostMtlsIntercept` obligations; the worker uses `local` as
/// `Routed::Inbound { orig_dst }`). Both legs are bound through
/// `HostMtlsIntercept::bind_transparent`; `converge_shared` installs the node
/// program at their targets and `install_inbound` registers the virtual
/// address as a destination member, so the production shared program — not a
/// test-installed rule — performs the divert.
///
/// Mutation targets: a listener that reports leg C's bound address as `local`
/// (every inbound connection mis-routed), and one that loses the sender.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_host_listener_reports_a_redirected_inbound_virtual_address_as_local() {
    assert!(is_root(), "S-ND295-70 host-listener evidence requires root and CAP_NET_ADMIN");
    let _kernel_lock = KernelStateLock::acquire();
    let _sandbox = HostListenerSandbox::fresh();

    let host = HostMtlsIntercept::new();
    let leg_f = host
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent binds leg F");
    let leg_c = Arc::new(
        host.bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("HostMtlsIntercept::bind_transparent binds leg C"),
    );
    let outbound_target = leg_f.bound_v4().expect("leg F reports its bound IPv4 address");
    let inbound_target = leg_c.bound_v4().expect("leg C reports its bound IPv4 address");
    let node_guard = host
        .converge_shared(None, outbound_target, inbound_target)
        .expect("the owned shared program installs on a clean kernel");
    let destination = host
        .install_inbound(HOST_LISTENER_INBOUND_VIRT, inbound_target.port())
        .expect("the virtual address is registered as an inbound destination member");
    eprintln!(
        "[S-ND295-70][inbound] leg F {outbound_target}; leg C {inbound_target}; virtual address \
         {HOST_LISTENER_INBOUND_VIRT}"
    );

    let client = std::thread::spawn(|| -> std::io::Result<(TcpStream, SocketAddrV4)> {
        let stream = dial(HOST_LISTENER_INBOUND_VIRT, Duration::from_secs(5))?;
        match stream.local_addr()? {
            std::net::SocketAddr::V4(source) => Ok((stream, source)),
            std::net::SocketAddr::V6(source) => {
                Err(std::io::Error::other(format!("the inbound client bound IPv6 {source}")))
            }
        }
    });
    let accepted = accept_leg_within(&leg_c, Duration::from_secs(8));
    let dialled = client.join().expect("the inbound client thread completes");
    let (stream, peer, local) = accepted.expect(
        "the redirected inbound dial must be accepted on leg C within 8 s; a timeout means the \
         shared program did not divert it",
    );
    let (client_stream, source) =
        dialled.expect("the inbound dial connects through the shared program's divert");
    eprintln!("[S-ND295-70][inbound] accepted peer={peer} local={local}; client source={source}");

    assert_eq!(
        local, HOST_LISTENER_INBOUND_VIRT,
        "the accepted connection's local address is the virtual address the sender dialled"
    );
    assert_eq!(peer, source, "the accepted peer is the sender's own address and port");

    drop((stream, client_stream));
    drop(destination);
    drop(node_guard);
    drop((leg_c, leg_f));
}

/// The host obligation for a listener the kernel has closed: when the
/// listening socket is destroyed from outside by a sock-diag destroy of its
/// exact tuple (`ss -K`, the S-ND295-63 stimulus), a pending accept wakes and
/// ends with the terminal `Accept` failure within 2 s; it never waits forever
/// (feature delta § *Driven port — intercept listener*, `accept` behaviour and
/// `HostMtlsIntercept` obligations). The accept is first shown to be pending:
/// no outcome arrives while nothing dials.
///
/// Mutation targets: an accept that ignores hang-up readiness (it waits
/// forever), and one that reports the loss as a connection-scoped
/// `OriginalDestination` failure (the worker would keep a dead task).
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_destroyed_host_listener_ends_its_pending_accept_with_an_accept_failure() {
    assert!(is_root(), "S-ND295-70 host-listener evidence requires root and CAP_NET_ADMIN");

    let host = HostMtlsIntercept::new();
    let leg = Arc::new(
        host.bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("HostMtlsIntercept::bind_transparent binds the leg"),
    );
    let bound = leg.bound_v4().expect("the leg reports its bound IPv4 address");
    let pending = spawn_accept_leg(&leg).expect("start the pending accept");
    match pending.recv_timeout(Duration::from_millis(300)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("with nothing dialling, the accept must still be pending: {other:?}"),
    }

    let destroyed = Command::new("ss")
        .args(["-K", "-t", "-l", "src", &bound.to_string()])
        .output()
        .expect("spawn ss -K for the listening tuple");
    let destroyed_listing = String::from_utf8_lossy(&destroyed.stdout);
    eprintln!(
        "[S-ND295-70][destroy] ss -K src {bound} -> {:?}\nstdout:\n{destroyed_listing}\nstderr:\n{}",
        destroyed.status.code(),
        String::from_utf8_lossy(&destroyed.stderr).trim()
    );
    assert!(destroyed.status.success(), "ss -K must succeed for the listening tuple {bound}");
    assert!(
        destroyed_listing.contains(&bound.to_string()),
        "ss -K must report destroying the listening tuple {bound}"
    );

    let outcome = pending
        .recv_timeout(Duration::from_secs(2))
        .expect("a destroyed listener must end the pending accept within 2 s");
    let failure = outcome.expect_err("a destroyed listener accepts nothing");
    assert!(
        matches!(accept_failure_of(&failure), Some(InterceptAcceptError::Accept { .. })),
        "the pending accept ends with the terminal Accept failure, got {failure:?}"
    );
}

/// The host obligation for an accepted descriptor: it is in the state
/// `std::net::TcpListener::accept` gives it today — `O_NONBLOCK` clear and
/// `FD_CLOEXEC` set — so `HostMtlsEnforcement` and the cleartext pass-through
/// receive the descriptor they receive today, and it never leaks into a child
/// process (feature delta § *Driven port — intercept listener*,
/// `HostMtlsIntercept` obligations; OBL-295-CLOEXEC).
///
/// Mutation targets: an accept that hands on the reactor's non-blocking
/// descriptor, and one that accepts without close-on-exec.
///
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-70 — The node's protection listeners belong to the protection port: a
/// simulated node opens no socket, and a listener stops when its wait is cancelled.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_accepted_connection_is_blocking_and_close_on_exec() {
    assert!(is_root(), "S-ND295-70 host-listener evidence requires root and CAP_NET_ADMIN");

    let host = HostMtlsIntercept::new();
    let leg = Arc::new(
        host.bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("HostMtlsIntercept::bind_transparent binds the leg"),
    );
    let bound = leg.bound_v4().expect("the leg reports its bound IPv4 address");
    let client = std::thread::spawn(move || dial(bound, Duration::from_secs(5)));
    let accepted = accept_leg_within(&leg, Duration::from_secs(8));
    let client_stream = client
        .join()
        .expect("the client thread completes")
        .expect("the client connects to the bound leg");
    let (stream, peer, local) = accepted.expect("the dial is accepted within 8 s");
    let client_source = match client_stream.local_addr().expect("the client's source address") {
        std::net::SocketAddr::V4(source) => source,
        other => panic!("the client bound a non-IPv4 source {other}"),
    };
    assert_eq!(local, bound, "a direct dial's accepted local address is the bound leg");
    assert_eq!(peer, client_source, "the accepted peer is the client");

    // SAFETY: `stream` owns a live accepted socket descriptor; F_GETFL and
    // F_GETFD read its flags and change nothing.
    let (status_flags, descriptor_flags) = unsafe {
        (
            libc::fcntl(stream.as_raw_fd(), libc::F_GETFL),
            libc::fcntl(stream.as_raw_fd(), libc::F_GETFD),
        )
    };
    assert!(status_flags >= 0, "F_GETFL: {}", std::io::Error::last_os_error());
    assert!(descriptor_flags >= 0, "F_GETFD: {}", std::io::Error::last_os_error());
    eprintln!(
        "[S-ND295-70][descriptor] accepted fd {} status flags {status_flags:#x} descriptor flags \
         {descriptor_flags:#x}",
        stream.as_raw_fd()
    );
    assert_eq!(status_flags & libc::O_NONBLOCK, 0, "the accepted descriptor is blocking");
    assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0, "the accepted descriptor is close-on-exec");

    drop((stream, client_stream));
}
