//! Tier-3 acceptance test for step 04-04 — adopt-on-restart boot recovery
//! (transparent-mtls-enrollment, D-TME-12 "Amended 2026-06-18 (02-06
//! adopt-on-restart)", §1–§4).
//!
//! Drives the PRODUCTION boot-recovery seam `veth_provisioner::
//! adopt_on_restart_recovery` — the observe → adopt → GC pass `run_server`
//! runs (after `AppState`, before the convergence loop, gated by
//! `mtls_worker.is_some()`) to rebuild the lost in-RAM `NetSlotAllocator` map
//! from the surviving slot↔alloc bindings and reap orphan netns. This is the
//! SAME pattern `serve_boot_provisions_veth` uses: drive the boot-pass seam
//! directly (its public signature IS the driving port), not the full
//! `run_server` (TLS / ports / mTLS-probe composition are out of scope for this
//! invariant).
//!
//! Litmus — what reds when the behaviour regresses: this test pins the SEAM's
//! observable kernel/ns effects (adopt the survivor's slot, keep the live
//! survivor netns, GC the orphan), NOT the `run_server` call-site wiring.
//! Deleting the call site would NOT turn this test RED — the test drives
//! `adopt_on_restart_recovery(...)` directly. (The wiring's happy path is
//! exercised by the mTLS-enabled `run_server` Tier-3 tests that boot with
//! `compose_mtls` true; its error→`health.startup.refused` arms — `NetnsRecovery`
//! / `NftRuleSweep` — are wired in `lib.rs` from these seam errors but have no
//! dedicated executing coverage, the known seam-vs-call-site gap.) Falsify the
//! seam itself by reverting `adopt`/`plan_adopt_actions`: assertion (b) reds
//! because the empty allocator hands slot `S` to the fresh alloc and the
//! survivor collides.
//!
//! THE HAZARD (verified ground truth, SPIKE-A/B/C, kernel 7.0.0):
//!   On a `serve` restart the in-RAM `NetSlotAllocator` map is reconstructed
//!   EMPTY, but workloads SURVIVE (setsid + kill_on_drop(false) + own cgroup
//!   scope). A naive empty allocator hands smallest-free slot 0 to the next NEW
//!   alloc → collides with a survivor still occupying `ovd-ns-0000` (B1
//!   resurrected across restart). Plus orphan-netns leak (a pre-restart
//!   `ovd-ns-<slot>` whose workload died in the restart window). B3: the netns
//!   name carries NO alloc identity, so the binding is RECOVERED via
//!   cgroup→PID→`/proc/<pid>/ns/net` inode correlation.
//!
//! The scenario `serve_restart_readopts_surviving_slot_and_gcs_orphan_netns`:
//!   1. Stand up a SURVIVOR: provision the slot-`S` netns + a real spawned PID
//!      living inside it, enrolled in the alloc's cgroup scope, with a Running
//!      `AllocStatusRow` (exactly the post-restart survivor shape).
//!   2. Stand up an ORPHAN: provision a slot-`O` netns with NO live PID (the
//!      restart-window-death leak), NO Running row.
//!   3. Restart: a FRESH empty `NetSlotAllocator` (the lost in-RAM map).
//!   4. Run the recovery pass against the fresh allocator + obs + cgroup root.
//!   5. Assert observable kernel/ns side effects:
//!      a. the SURVIVOR netns is KEPT (`ip netns identify <survivor pid>` still
//!         resolves it) — recovery never tore down a live survivor;
//!      b. the survivor slot `S` is ADOPTED — a fresh `assign` for a NEW alloc
//!         does NOT return slot `S` (the cross-restart B1 collision is closed);
//!      c. the ORPHAN netns is GONE (`ip netns list` no longer shows it).
//!
//! Every assertion is load-bearing (fails if the behaviour regresses): (a)
//! reds if recovery tears down survivors; (b) reds if `adopt` did not claim the
//! slot (the empty-allocator hands `S` to the new alloc → collision); (c) reds
//! if orphan-GC did not run. No vacuous checks (the 02-02 sysctl / 03-03 F5
//! trap).
//!
//! Root + CAP_NET_ADMIN + cgroup-write required (real `ip netns` + real spawn +
//! real cgroup scope); SKIP on an unprivileged runner. Run via `cargo xtask
//! lima run -- cargo nextest run -p overdrive-control-plane --features
//! integration-tests`. NEVER `--no-run` — a compile-only gate is green even
//! when every fixture refuses at boot.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege messages are the legitimate way these Tier-3 tests
// communicate "capability absent, scenario skipped" on an unprivileged runner.
#![allow(clippy::print_stderr)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// One sequential boot-recovery walkthrough whose kernel assertions exceed the
// line budget; splitting it would scatter one scenario across helpers.
#![allow(clippy::too_many_lines)]
// `ovd-ns-<slot>` / `AllocStatusRow` etc. read as prose identifiers.
#![allow(clippy::doc_markdown)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::veth_provisioner::{
    NetSlot, NetSlotAllocator, WorkloadNetnsPlan, adopt_on_restart_recovery,
    derive_workload_netns_plan, provision_workload_netns, responder_addr_for_slot,
    teardown_workload_netns,
};
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// True iff this process is uid 0 (root). The netns provision + cgroup scope
/// create + spawn-into-netns path all need CAP_NET_ADMIN / CAP_SYS_ADMIN.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// `ip netns identify <pid>` → the netns NAME the PID lives in.
fn netns_identify(pid: u32) -> Option<String> {
    let out = Command::new("ip").args(["netns", "identify", &pid.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
}

/// `ip netns list` contains `<netns>` (first whitespace-delimited token).
fn netns_present(netns: &str) -> bool {
    let out = Command::new("ip").args(["netns", "list"]).output().expect("spawn ip netns list");
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.split_whitespace().next() == Some(netns))
}

/// `nft -a list chain ip overdrive-mtls prerouting` — the live shared
/// `prerouting` chain dump (with handles). The AT inspects the OBSERVABLE
/// kernel state through the same `nft` surface the production guard's `Drop`
/// and the §5 sweep operate on. `None` iff the chain (or table) does not exist.
fn nft_chain_dump() -> Option<String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", "overdrive-mtls", "prerouting"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// True iff the live chain dump carries an egress per-workload TPROXY rule for
/// `host_veth` → `127.0.0.1:<port>` (the `iifname "<veth>"` match conjoined with
/// the `tproxy to 127.0.0.1:<port>` redirect on one line — the exact shape
/// `install_outbound_tproxy` appends and the §5 sweep removes). The AT re-derives
/// the predicate locally (the production parse is private to `mtls_intercept`).
fn dump_has_egress_rule(dump: &str, host_veth: &str, port: u16) -> bool {
    let iifname = format!("iifname \"{host_veth}\"");
    let redirect = format!("tproxy to 127.0.0.1:{port}");
    dump.lines().any(|l| l.contains(&iifname) && l.contains(&redirect))
}

/// True iff the live chain dump carries the F5 `meta mark … accept` leg-S-dial
/// exemption (nft's zero-padded `0x000000NN` rendering). This is the SHARED
/// infra the §5 sweep MUST leave untouched.
fn dump_has_leg_s_exemption(dump: &str) -> bool {
    let leg_s_mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    let nft_rendered = format!("meta mark {leg_s_mark:#010x} accept");
    dump.lines().any(|l| l.trim().contains(&nft_rendered))
}

/// RAII best-effort cleanup of the shared `overdrive-mtls` nft table + the
/// fwmark `ip rule` / `local` route, so a panicking §5 test leaves no residue
/// for the next run (the sweep test deliberately strands a guard-less rule).
struct SharedInfraGuard;
impl Drop for SharedInfraGuard {
    fn drop(&mut self) {
        let _ = Command::new("nft").args(["delete", "table", "ip", "overdrive-mtls"]).status();
    }
}

/// RAII teardown — runs the production teardown for a slot plan on drop so the
/// netns + host veth leave no residue even when an assertion panics.
struct NetnsGuard {
    plan: WorkloadNetnsPlan,
}
impl Drop for NetnsGuard {
    fn drop(&mut self) {
        let _ = teardown_workload_netns(&self.plan);
    }
}

/// RAII kill — reaps a spawned `/bin/sleep` survivor PID + its cgroup scope on
/// drop (it lives in its own netns + scope, detached from this process).
struct PidGuard {
    pid: u32,
    scope: PathBuf,
}
impl Drop for PidGuard {
    fn drop(&mut self) {
        // SAFETY: kill is always safe; SIGKILL the survivor, ignore ESRCH.
        unsafe {
            libc::kill(self.pid.cast_signed(), libc::SIGKILL);
        }
        // Reap the cgroup scope: mass-kill any residents, then rmdir.
        let _ = std::fs::write(self.scope.join("cgroup.kill"), "1");
        let _ = std::fs::remove_dir(&self.scope);
    }
}

fn plan_for(slot: u16) -> WorkloadNetnsPlan {
    let s = NetSlot::new(slot).expect("slot in range");
    derive_workload_netns_plan(s, responder_addr_for_slot(s))
}

/// Ensure the production `overdrive.slice/workloads.slice` cgroup hierarchy
/// exists (the alloc scope is a child of it). Mirrors the boot path's
/// `create_workloads_slice_with_controllers`. Returns Ok iff the slice is
/// usable (the recovery pass reads `<alloc>.scope/cgroup.procs` under it).
async fn ensure_workloads_slice(cgroup_root: &Path) -> bool {
    use overdrive_worker::cgroup_manager::CgroupManager;
    let fs: Arc<dyn overdrive_core::traits::CgroupFs> =
        Arc::new(overdrive_host::RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs)
        .create_workloads_slice_with_controllers()
        .await
        .is_ok()
}

/// Spawn a long-lived `/bin/sleep` INSIDE `netns`, enrolled in `scope`, so it
/// is the post-restart survivor: a live PID whose `/proc/<pid>/ns/net` inode
/// resolves to the slot netns AND whose pid is in the alloc's `cgroup.procs`.
/// The scope cgroup dir is created (mkdir under the delegated workloads.slice)
/// before the shell enrols its own pid into it. Returns the spawned PID.
fn spawn_survivor_in_netns(netns: &str, scope: &Path) -> u32 {
    // Create the scope cgroup (a plain `mkdir` under the delegated
    // workloads.slice — cgroup v2 materialises `cgroup.procs` automatically).
    let mk = Command::new("mkdir").args(["-p", &scope.to_string_lossy()]).status();
    assert!(
        mk.is_ok_and(|s| s.success()) && scope.join("cgroup.procs").exists(),
        "alloc cgroup scope {} must be creatable with a live cgroup.procs (cgroup delegation)",
        scope.display(),
    );
    // Spawn a long-lived `sleep` INTO the netns. `ip netns exec` enters a fresh
    // MOUNT namespace where `/sys` is remounted for the netns — so the cgroup
    // write must NOT be done inside the exec (the netns's `/sys/fs/cgroup` lacks
    // the workload hierarchy). Instead: spawn the sleep in the netns, then
    // enroll its PID into the scope from the HOST mount namespace below.
    let child = Command::new("ip")
        .args(["netns", "exec", netns, "setsid", "sleep", "3600"])
        .spawn()
        .expect("spawn survivor in netns");
    let pid = child.id();
    // The survivor is INTENTIONALLY detached — it must outlive this process
    // (the post-restart-survivor shape). `std::mem::forget` the handle so it is
    // never `wait()`ed/reaped on drop; PidGuard reaps it by PID at test end.
    std::mem::forget(child);
    // Enroll the survivor PID into the alloc's cgroup scope from the HOST mount
    // namespace (where `/sys/fs/cgroup/overdrive.slice/...` is the real
    // hierarchy). This is exactly what the production ExecDriver does after
    // spawning a workload — the recovery pass reads this `cgroup.procs`.
    let procs = scope.join("cgroup.procs");
    std::fs::write(&procs, format!("{pid}\n"))
        .unwrap_or_else(|e| panic!("enroll survivor pid {pid} into {}: {e}", procs.display()));
    pid
}

/// Build a single-peer in-process obs store + write a Running row for `alloc`
/// (the survivor's post-restart observation — the recovery pass reads the
/// Running set from here, per §1 step 2).
async fn obs_with_running(alloc: &AllocationId) -> Arc<dyn ObservationStore> {
    let node_id = NodeId::new("node-001").expect("node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("svc-aor").expect("workload id"),
        node_id: node_id.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id.clone() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write Running row");
    obs
}

#[tokio::test]
async fn serve_restart_readopts_surviving_slot_and_gcs_orphan_netns() {
    // CROSS-TEST GC SAFETY (host-global `ip netns` namespace).
    //
    // `adopt_on_restart_recovery` → `adopt_observe` enumerates EVERY `ovd-ns-*`
    // netns on the host and GCs any whose slot is not owned by a Running alloc in
    // THIS test's obs store (here `obs_with_running(&survivor_alloc)` — only the
    // survivor). The netns namespace is process-global, so a sibling Tier-3 test
    // holding a live `ovd-ns-<other-slot>` at the recovery instant would, in
    // principle, be classified as an orphan and torn down.
    //
    // This hazard is now STRUCTURALLY guarded, not merely timing-lucky: commit
    // `64c05b32` added all six per-alloc-netns Tier-3 tests (this one and the
    // `alloc_netns_lifecycle` family) to the `host-kernel-shared`
    // `max-threads = 1` test-group in `.config/nextest.toml` (verified via
    // `nextest show-config test-groups`). That group is the single-writer
    // cross-PROCESS guard, so no sibling test holds a live `ovd-ns-<other-slot>`
    // while this test's recovery pass runs its global netns scan — the
    // false-orphan GC cannot interleave. (Before `64c05b32` the suite passed
    // empirically — full suite GREEN on kernel 7.0.0 plus a 5/5 `--test-threads
    // 8` stress — but that was a TIMING observation; the serialization makes it
    // a guarantee.) See reviews/04-04.md cross-test GC item.

    // Choose distinct slots well away from 0 so a fresh-assign collision is
    // unambiguous: survivor at slot S, orphan at slot O. The fresh allocator
    // would hand smallest-free (slot 0) to a new alloc UNLESS S is adopted —
    // the test saturates 0..S so the smallest free is EXACTLY S, making the
    // adoption load-bearing: if `adopt(S)` did NOT run, the fresh assign
    // returns S and collides with the survivor.
    const SURVIVOR_SLOT: u16 = 7;
    const ORPHAN_SLOT: u16 = 9;

    if !is_root() {
        eprintln!(
            "SKIP serve_restart_readopts_surviving_slot_and_gcs_orphan_netns: not root \
             (needs CAP_NET_ADMIN + CAP_SYS_ADMIN for ip netns + spawn-into-netns + cgroup)"
        );
        return;
    }

    let cgroup_root = PathBuf::from("/sys/fs/cgroup");

    let survivor_alloc = AllocationId::new("aor-survivor").expect("alloc id");
    let survivor_plan = plan_for(SURVIVOR_SLOT);
    let orphan_plan = plan_for(ORPHAN_SLOT);
    let survivor_scope =
        cgroup_root.join("overdrive.slice/workloads.slice").join(format!("{survivor_alloc}.scope"));

    // Pre-sweep residue from a crashed prior run.
    let _ = teardown_workload_netns(&survivor_plan);
    let _ = teardown_workload_netns(&orphan_plan);
    let _ = std::fs::write(survivor_scope.join("cgroup.kill"), "1");
    let _ = std::fs::remove_dir(&survivor_scope);

    // The recovery pass correlates each Running alloc's `cgroup.procs` PIDs to
    // their netns — so the survivor MUST be enrolled in
    // `overdrive.slice/workloads.slice/<alloc>.scope`. Build that hierarchy
    // (the boot path's own `create_workloads_slice_with_controllers`) first.
    if !ensure_workloads_slice(&cgroup_root).await {
        eprintln!(
            "SKIP serve_restart_readopts_surviving_slot_and_gcs_orphan_netns: \
             workloads.slice bootstrap failed (likely no cgroup delegation)"
        );
        return;
    }

    // --- (1) Provision the SURVIVOR netns; arm RAII guards. ---
    if let Err(source) = provision_workload_netns(&survivor_plan) {
        eprintln!(
            "SKIP serve_restart_readopts_surviving_slot_and_gcs_orphan_netns: \
             survivor provision failed (likely no CAP_NET_ADMIN): {source}"
        );
        return;
    }
    let _survivor_netns_guard = NetnsGuard { plan: survivor_plan.clone() };

    let survivor_pid = spawn_survivor_in_netns(&survivor_plan.netns, &survivor_scope);
    let _pid_guard = PidGuard { pid: survivor_pid, scope: survivor_scope.clone() };
    // Give the shell a moment to enrol + exec.
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Sanity precondition (NOT the assertion under test): the survivor PID lives
    // in the survivor netns. If this fails the fixture is broken — skip.
    if netns_identify(survivor_pid).as_deref() != Some(survivor_plan.netns.as_str()) {
        eprintln!(
            "SKIP serve_restart_readopts_surviving_slot_and_gcs_orphan_netns: \
             survivor PID {survivor_pid} did not land in {} (fixture precondition)",
            survivor_plan.netns
        );
        return;
    }

    // --- (2) Provision the ORPHAN netns (no live PID, no Running row). ---
    provision_workload_netns(&orphan_plan).expect("orphan provision");
    let _orphan_netns_guard = NetnsGuard { plan: orphan_plan.clone() };

    // --- (3) Restart: a FRESH empty allocator (the lost in-RAM map). ---
    let allocator = NetSlotAllocator::new();
    let obs = obs_with_running(&survivor_alloc).await;

    // --- (4) Run the production boot-recovery pass. ---
    adopt_on_restart_recovery(obs.as_ref(), &allocator, &cgroup_root)
        .await
        .expect("recovery pass must succeed (survivor adopts cleanly, orphan GCs)");

    // --- (5a) The SURVIVOR netns is KEPT — recovery never tore down a live
    // survivor (reds if recovery GC'd the survivor by mistake). ---
    assert_eq!(
        netns_identify(survivor_pid).as_deref(),
        Some(survivor_plan.netns.as_str()),
        "survivor PID {survivor_pid} must still live in its netns {} after recovery \
         (recovery must ADOPT the live survivor, never reap it)",
        survivor_plan.netns,
    );

    // --- (5b) The survivor slot S is ADOPTED: it is held by the survivor, and
    // a fresh `assign` for a NEW alloc does NOT return slot S. To make this
    // load-bearing, saturate 0..SURVIVOR_SLOT so the smallest-free is EXACTLY
    // S — if `adopt(S)` did not run, `assign` would return S and collide. ---
    assert_eq!(
        allocator.snapshot().get(&survivor_alloc).copied(),
        Some(NetSlot::new(SURVIVOR_SLOT).expect("slot in range")),
        "recovery must ADOPT slot {SURVIVOR_SLOT} for the survivor (rebuild the lost map)",
    );
    for s in 0..SURVIVOR_SLOT {
        let filler = AllocationId::new(&format!("aor-filler-{s}")).expect("alloc id");
        let got = allocator.assign(filler).expect("filler assign under capacity");
        assert_ne!(
            got,
            NetSlot::new(SURVIVOR_SLOT).expect("slot in range"),
            "a filler assign must not be handed the adopted survivor slot {SURVIVOR_SLOT}",
        );
    }
    let fresh = AllocationId::new("aor-fresh").expect("alloc id");
    let fresh_slot = allocator.assign(fresh).expect("fresh assign");
    assert_ne!(
        fresh_slot,
        NetSlot::new(SURVIVOR_SLOT).expect("slot in range"),
        "a fresh assign after recovery must NOT collide with the adopted survivor slot \
         {SURVIVOR_SLOT} (the cross-restart B1 collision the adopt pass closes)",
    );

    // --- (5c) The ORPHAN netns is GONE (orphan-GC ran). ---
    assert!(
        !netns_present(&orphan_plan.netns),
        "the orphan netns {} (no live PID) must be reaped by recovery's orphan-GC",
        orphan_plan.netns,
    );

    // Drop guards reap the survivor PID + both netns + the cgroup scope.
}

/// Tier-3 acceptance test for step 04-04 §5 — the surviving per-workload
/// nft-TPROXY rule sweep (D-TME-12 §5, folding 03-01 review finding D2).
///
/// THE HAZARD (verified ground truth, SPIKE-D, kernel 7.0.0): a per-workload
/// egress TPROXY rule is APPENDED to the node-global shared `overdrive-mtls`
/// `prerouting` chain and is NEVER torn down per-workload, so it SURVIVES a
/// `serve` restart — but its in-RAM RAII `TproxyInterceptGuard` is LOST (the CP
/// died; `Drop` never ran). The surviving rule redirects to a now-dead leg-F
/// listener port → DEAD weight; a later per-alloc re-install with a NEW
/// ephemeral leg-F port does NOT match `(veth, oldPort)` and APPENDS A SECOND
/// rule (the duplicate-stack D2 hazard). §5's boot-recovery sweep removes EVERY
/// per-workload rule so the next re-install starts from a clean chain.
///
/// The scenario `serve_restart_sweeps_surviving_per_workload_tproxy_rule`:
///   1. Stand up the SHARED infra + a per-workload egress rule by calling the
///      production `install_outbound_tproxy(host_veth, port)` (its append IS the
///      pre-restart state; it ensures the table+chain+F5 exemption idempotently
///      and appends one `iifname`-matched egress rule).
///   2. Simulate CP DEATH: `std::mem::forget` the returned guard so its `Drop`
///      NEVER runs against the kernel — the rule SURVIVES guard-less, exactly the
///      post-restart survivor shape SPIKE-D proved.
///   3. Run the PRODUCTION boot-recovery sweep
///      `mtls_intercept::sweep_per_workload_tproxy_rules()`.
///   4. Assert observable kernel side effects (every one load-bearing):
///      a. the surviving per-workload egress rule is GONE from the post-sweep
///         chain (reds if the sweep did not remove it — the D2 dead-weight);
///      b. the F5 `meta mark … accept` exemption REMAINS (reds if the sweep over-
///         reached and tore down shared infra — the SCOPE GUARD: sweep is
///         cleanup, NEVER shared-infra teardown);
///      c. the table+chain REMAIN (the dump still resolves);
///      d. the sweep RETURNS the count of rules it removed (== 1 here);
///      e. a subsequent clean re-install appends EXACTLY ONE rule (the chain was
///         left clean → no duplicate-stack).
///   5. Idempotent re-sweep returns 0 (a chain with only shared infra is a
///      no-op).
///
/// PORT-TO-PORT: drives the production `sweep_per_workload_tproxy_rules` free
/// function — its public signature IS the driving port for the node-global
/// chain. Litmus: revert the sweep BODY (the `per_workload_rule_handles_in_dump`
/// classify + by-handle delete) ⇒ assertion (a)/(d) stay RED. This test drives
/// the sweep seam directly, so it does NOT exercise (and would NOT red on
/// deleting) the `run_server` call-site wiring — that wiring's happy path is
/// covered by the mTLS-enabled `run_server` Tier-3 boots, and its
/// error→`health.startup.refused` arm (`NftRuleSweep`) is the known seam-vs-
/// call-site gap. This test does NOT assert that survivor egress interception is
/// restored after restart — that is the ACCEPTED #26-coupled limitation (a
/// still-Running survivor legitimately ends with NO nft rule until reschedule),
/// explicitly OUT of §5 scope (cleanup, not restoration).
///
/// Root + CAP_NET_ADMIN required (real `nft` table/chain/rule); SKIP on an
/// unprivileged runner. Run via `cargo xtask lima run -- cargo nextest run -p
/// overdrive-control-plane --features integration-tests`. NEVER `--no-run`.
#[tokio::test]
async fn serve_restart_sweeps_surviving_per_workload_tproxy_rule() {
    use overdrive_worker::mtls_intercept::{
        install_outbound_tproxy, sweep_per_workload_tproxy_rules,
    };

    // A synthetic host-veth NAME for the egress rule's `iifname` match. The
    // egress install stores the rule by string match regardless of whether the
    // interface exists, so no real veth is needed to exercise the sweep — the
    // sweep operates on the chain dump, not on live interfaces.
    const HOST_VETH: &str = "ovh-sweep0";
    // A leg-F redirect port standing in for the (now-dead) listener the survivor
    // rule points at.
    const DEAD_LEG_F_PORT: u16 = 45123;

    if !is_root() {
        eprintln!(
            "SKIP serve_restart_sweeps_surviving_per_workload_tproxy_rule: not root \
             (needs CAP_NET_ADMIN for nft table/chain/rule)"
        );
        return;
    }

    // Pre-sweep residue from a crashed prior run: drop the whole shared table so
    // we start from a known-empty kernel (the install re-creates it idempotently).
    let _ = Command::new("nft").args(["delete", "table", "ip", "overdrive-mtls"]).status();
    let _infra_guard = SharedInfraGuard;

    // --- (1) Pre-restart state: ensure shared infra + ONE per-workload egress
    // rule via the production install. ---
    let guard = match install_outbound_tproxy(HOST_VETH, DEAD_LEG_F_PORT) {
        Ok(g) => g,
        Err(source) => {
            eprintln!(
                "SKIP serve_restart_sweeps_surviving_per_workload_tproxy_rule: \
                 install_outbound_tproxy failed (likely no CAP_NET_ADMIN / nft absent): {source}"
            );
            return;
        }
    };

    // Precondition (NOT the assertion under test): the egress rule + the F5
    // exemption are both present pre-sweep. If not, the fixture is broken — skip.
    let pre = nft_chain_dump().expect("shared chain must exist after install");
    if !dump_has_egress_rule(&pre, HOST_VETH, DEAD_LEG_F_PORT) || !dump_has_leg_s_exemption(&pre) {
        eprintln!(
            "SKIP serve_restart_sweeps_surviving_per_workload_tproxy_rule: \
             pre-sweep chain missing egress rule or F5 exemption (fixture precondition)"
        );
        return;
    }

    // --- (2) Simulate CP DEATH: the guard's Drop must NEVER run, so the rule
    // SURVIVES guard-less (the post-restart survivor shape, SPIKE-D). ---
    std::mem::forget(guard);

    // --- (3) Run the PRODUCTION boot-recovery sweep. ---
    let swept = sweep_per_workload_tproxy_rules().expect("sweep must succeed against a live chain");

    // --- (4d) The sweep removed exactly the one surviving per-workload rule. ---
    assert_eq!(
        swept, 1,
        "the sweep must report removing exactly the 1 surviving per-workload egress rule",
    );

    let post = nft_chain_dump().expect("shared chain must still exist after the sweep");

    // --- (4a) The surviving per-workload egress rule is GONE. ---
    assert!(
        !dump_has_egress_rule(&post, HOST_VETH, DEAD_LEG_F_PORT),
        "the surviving per-workload egress rule for {HOST_VETH} → 127.0.0.1:{DEAD_LEG_F_PORT} \
         must be SWEPT from the shared chain (the D2 dead-weight redirecting to a dead listener)",
    );

    // --- (4b) The F5 exemption REMAINS (sweep is cleanup, NOT shared-infra
    // teardown — the SCOPE GUARD). ---
    assert!(
        dump_has_leg_s_exemption(&post),
        "the shared F5 `meta mark … accept` exemption must REMAIN after the sweep \
         (the sweep removes only per-workload rules, never shared infra)",
    );

    // --- (4c) The table+chain REMAIN (the dump resolved at all above proves the
    // chain survived; assert the chain header is intact). ---
    assert!(
        post.contains("chain prerouting"),
        "the shared table+chain must survive the sweep (only per-workload rules are removed)",
    );

    // --- (4e) A subsequent clean re-install appends EXACTLY ONE rule (the chain
    // was left clean → no duplicate-stack — the forward-correctness §5 buys). ---
    let reinstall = install_outbound_tproxy(HOST_VETH, DEAD_LEG_F_PORT)
        .expect("clean re-install against the swept chain must succeed");
    let after_reinstall = nft_chain_dump().expect("chain present after re-install");
    let egress_rule_count = after_reinstall
        .lines()
        .filter(|l| {
            l.contains(&format!("iifname \"{HOST_VETH}\""))
                && l.contains(&format!("tproxy to 127.0.0.1:{DEAD_LEG_F_PORT}"))
        })
        .count();
    assert_eq!(
        egress_rule_count, 1,
        "a clean re-install after the sweep must append EXACTLY ONE egress rule \
         (no duplicate-stack — the chain was swept clean)",
    );
    // This re-install's guard IS dropped (normal teardown) — its Drop removes the
    // one rule by handle, leaving only shared infra for the idempotent re-sweep.
    drop(reinstall);

    // --- (5) Idempotent re-sweep: a chain carrying ONLY shared infra is a
    // no-op (0 rules removed). ---
    let swept_again =
        sweep_per_workload_tproxy_rules().expect("re-sweep against a clean chain must succeed");
    assert_eq!(
        swept_again, 0,
        "a re-sweep of a chain carrying only shared infra must remove 0 rules (idempotent no-op)",
    );

    // SharedInfraGuard drops the whole table at test end.
}
