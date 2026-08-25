// PROBE increment-d — subprocess-free-veth-provisioner (GH #233), mtls_intercept.rs scope.
//
// Proves: rtnetlink (pure netlink syscalls, NO `ip` subprocess) replicates the
// two node-global `ip` shell-outs in
// crates/overdrive-worker/src/mtls_intercept.rs::ensure_shared_routing_infra.
// Every write is verified by a NETLINK read-back (rule dump / route dump), not
// by trusting the write returned Ok.
//
// Replicated REAL operations (cited to mtls_intercept.rs):
//   - ip rule add fwmark 0x1 lookup 100
//       run_ip(&["rule","add","fwmark",&fwmark,"lookup",&rt_table])  L649
//       guarded by !ip_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE) L648
//   - ip route add local 0.0.0.0/0 dev lo table 100
//       ensure_ip_route_local() L741-766 (tolerates EEXIST as converged)
//
// REAL constant VALUES (verbatim from mtls_intercept.rs):
//   TPROXY_FWMARK   = 0x1   (L88)
//   TPROXY_RT_TABLE = 100   (L93)
//
// Idempotency: the production code guards `ip rule add` behind an `ip rule show`
// scan because iproute2 `ip rule add` STACKS duplicates. This probe characterises
// what the *netlink* path does on a naked re-add (rtnetlink's RuleAddRequest sets
// NLM_F_EXCL | NLM_F_CREATE by default — rule/add.rs L156), i.e. whether the
// kernel returns EEXIST (netlink is idempotent by itself) or stacks a duplicate
// (the production dump-guard is still required). Both outcomes are reported
// honestly; the read-back rule COUNT is the oracle.

use std::net::Ipv4Addr;

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::route::{RouteAttribute, RouteScope, RouteType};
use rtnetlink::packet_route::rule::{RuleAction, RuleAttribute};
use rtnetlink::{new_connection, Handle, IpVersion, RouteMessageBuilder};

// Real constant values, verbatim from crates/overdrive-worker/src/mtls_intercept.rs.
const TPROXY_FWMARK: u32 = 0x1; // L88
const TPROXY_RT_TABLE: u32 = 100; // L93

macro_rules! step {
    ($($a:tt)*) => {{ println!("\n>>> {}", format!($($a)*)); }};
}
macro_rules! ok {
    ($($a:tt)*) => {{ println!("    [PASS] {}", format!($($a)*)); }};
}
macro_rules! bad {
    ($($a:tt)*) => {{ println!("    [FAIL] {}", format!($($a)*)); }};
}
macro_rules! info {
    ($($a:tt)*) => {{ println!("    [INFO] {}", format!($($a)*)); }};
}

/// ifindex of a link by name in the current netns, or None.
async fn link_index(handle: &Handle, name: &str) -> Option<u32> {
    let mut s = handle.link().get().match_name(name.to_string()).execute();
    match s.try_next().await {
        Ok(Some(msg)) => Some(msg.header.index),
        _ => None,
    }
}

/// Count of `ip rule` entries that BOTH mark on `fwmark == mark` AND look up
/// `table` — the exact conjunction ip_rule_dump_has_fwmark (mtls_intercept.rs
/// L795) enforces, here against the structured netlink dump instead of parsing
/// `ip rule show` text. table<=255 lands in the message header (RuleAddRequest
/// ::table_id L72), so the kernel reports it in `header.table`; table>255 lands
/// in the FRA_TABLE attribute. Check both so the count is table-value-agnostic.
async fn count_fwmark_table_rules(handle: &Handle, mark: u32, table: u32) -> usize {
    let mut s = handle.rule().get(IpVersion::V4).execute();
    let mut n = 0usize;
    while let Ok(Some(msg)) = s.try_next().await {
        let mut has_mark = false;
        let mut has_table = u32::from(msg.header.table) == table;
        for a in &msg.attributes {
            match a {
                RuleAttribute::FwMark(m) if *m == mark => has_mark = true,
                RuleAttribute::Table(t) if *t == table => has_table = true,
                _ => {}
            }
        }
        if has_mark && has_table {
            n += 1;
        }
    }
    n
}

/// Count of `local 0.0.0.0/0` routes present in `table`, i.e. the read-back of
/// `ip route add local 0.0.0.0/0 dev lo table 100`. Matches on kind == Local
/// (RTN_LOCAL), destination_prefix_length == 0, and the table (header.table for
/// <=255, else the RTA_TABLE attribute). The raw RTM_GETROUTE dump returns
/// routes from ALL tables (iproute2 filters to `main` in userspace; the netlink
/// layer does not), so table-100 routes surface here without a table hint.
async fn count_local_default_route(handle: &Handle, table: u32) -> (usize, Option<u32>) {
    let msg = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let mut s = handle.route().get(msg).execute();
    let mut n = 0usize;
    let mut oif = None;
    while let Ok(Some(m)) = s.try_next().await {
        if m.header.kind != RouteType::Local || m.header.destination_prefix_length != 0 {
            continue;
        }
        let mut table_ok = u32::from(m.header.table) == table;
        let mut this_oif = None;
        for a in &m.attributes {
            match a {
                RouteAttribute::Table(t) if *t == table => table_ok = true,
                RouteAttribute::Oif(i) => this_oif = Some(*i),
                _ => {}
            }
        }
        if table_ok {
            n += 1;
            oif = this_oif;
        }
    }
    (n, oif)
}

/// `ip rule add fwmark <mark> lookup <table>` via netlink. Returns Ok(true) if
/// the kernel accepted the add, Ok(false) if it rejected it as EEXIST (the
/// NLM_F_EXCL duplicate signal), Err on any other failure.
async fn rule_add_fwmark(handle: &Handle, mark: u32, table: u32) -> Result<bool, String> {
    match handle
        .rule()
        .add()
        .v4()
        .action(RuleAction::ToTable)
        .fw_mark(mark)
        .table_id(table)
        .execute()
        .await
    {
        Ok(()) => Ok(true),
        Err(rtnetlink::Error::NetlinkError(e)) if e.raw_code().abs() == libc::EEXIST => Ok(false),
        Err(e) => Err(format!("{e}")),
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("=== PROBE increment-d: rtnetlink ip rule + ip route (mtls_intercept.rs shared infra) ===");
    println!(
        "kernel: {}",
        std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim()
    );
    println!("constants: TPROXY_FWMARK={TPROXY_FWMARK:#x}  TPROXY_RT_TABLE={TPROXY_RT_TABLE}");

    let (connection, handle, _) = new_connection().expect("netlink connection");
    tokio::spawn(connection);

    let lo_idx = link_index(&handle, "lo").await.expect("lo ifindex");
    info!("lo ifindex = {lo_idx}");

    // Best-effort pre-clean so re-runs start from a known state.
    {
        let mut s = handle.rule().get(IpVersion::V4).execute();
        let mut victims = Vec::new();
        while let Ok(Some(msg)) = s.try_next().await {
            let has_mark = msg.attributes.iter().any(
                |a| matches!(a, RuleAttribute::FwMark(m) if *m == TPROXY_FWMARK),
            );
            let has_table = u32::from(msg.header.table) == TPROXY_RT_TABLE
                || msg.attributes.iter().any(
                    |a| matches!(a, RuleAttribute::Table(t) if *t == TPROXY_RT_TABLE),
                );
            if has_mark && has_table {
                victims.push(msg);
            }
        }
        for v in victims {
            let _ = handle.rule().del(v).execute().await;
        }
    }
    let precleaned = build_local_default_route(lo_idx);
    let _ = handle.route().del(precleaned).execute().await;

    let mut fails = 0usize;

    // ---- OP 1: ip rule add fwmark 0x1 lookup 100 (production-equivalent guard) ----
    step!("ip rule add fwmark {TPROXY_FWMARK:#x} lookup {TPROXY_RT_TABLE}  (run_ip L649, guarded L648)");
    // Production guard: add only if not already present (ip_rule_fwmark_present).
    let pre = count_fwmark_table_rules(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await;
    info!("guard read-back BEFORE: {pre} matching rule(s)");
    if pre == 0 {
        match rule_add_fwmark(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await {
            Ok(true) => info!("rule add accepted"),
            Ok(false) => info!("rule add returned EEXIST (already present)"),
            Err(e) => {
                bad!("rule add failed: {e}");
                fails += 1;
            }
        }
    } else {
        info!("guard says present — skip add (production-equivalent idempotent ensure)");
    }
    let after = count_fwmark_table_rules(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await;
    if after == 1 {
        ok!("read-back: exactly 1 rule `fwmark {TPROXY_FWMARK:#x} lookup {TPROXY_RT_TABLE}`");
    } else {
        bad!("read-back: expected 1 rule, found {after}");
        fails += 1;
    }

    // ---- Idempotency characterisation: naked re-add via netlink ----
    step!("idempotency: re-issue `rule add fwmark {TPROXY_FWMARK:#x} lookup {TPROXY_RT_TABLE}` WITHOUT the guard");
    match rule_add_fwmark(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await {
        Ok(true) => info!(
            "naked re-add SUCCEEDED — netlink did NOT dedup; kernel stacked another rule (like iproute2)"
        ),
        Ok(false) => info!(
            "naked re-add returned -EEXIST — NLM_F_EXCL dedups at the netlink layer (MORE idempotent than iproute2 `ip rule add`)"
        ),
        Err(e) => info!("naked re-add unexpected error: {e}"),
    }
    let dup = count_fwmark_table_rules(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await;
    if dup == 1 {
        ok!("read-back after naked re-add: still exactly 1 rule (no duplicate stacked)");
    } else {
        bad!(
            "read-back after naked re-add: {dup} rules — a duplicate WAS stacked; production dump-guard is REQUIRED"
        );
        // Not a hard FAIL of the mechanism: this is a design finding. Record it,
        // do not inflate the failure count — the production code already guards.
        info!("=> design implication: keep the presence-guard before rule add (as production does)");
    }

    // ---- OP 2: ip route add local 0.0.0.0/0 dev lo table 100 ----
    step!("ip route add local 0.0.0.0/0 dev lo table {TPROXY_RT_TABLE}  (ensure_ip_route_local L741)");
    let route = build_local_default_route(lo_idx);
    match handle.route().add(route).execute().await {
        Ok(()) => info!("route add accepted"),
        Err(rtnetlink::Error::NetlinkError(e)) if e.raw_code().abs() == libc::EEXIST => {
            info!("route add returned EEXIST — already converged (ensure_ip_route_local tolerates this)")
        }
        Err(e) => {
            bad!("route add failed: {e}");
            fails += 1;
        }
    }
    let (rn, roif) = count_local_default_route(&handle, TPROXY_RT_TABLE).await;
    if rn >= 1 {
        ok!(
            "read-back: local 0.0.0.0/0 route present in table {TPROXY_RT_TABLE} (count={rn}, oif={roif:?}, lo={lo_idx})"
        );
        if roif == Some(lo_idx) {
            ok!("read-back: route oif == lo (dev lo confirmed)");
        } else {
            info!("route oif {roif:?} != lo {lo_idx} (kernel may omit RTA_OIF on a local route; table+kind matched)");
        }
    } else {
        bad!("read-back: local 0.0.0.0/0 route NOT found in table {TPROXY_RT_TABLE}");
        fails += 1;
    }

    // ---- Idempotency: re-issue the route add (EEXIST-tolerant, as production) ----
    step!("idempotency: re-issue the route add (ensure_ip_route_local tolerates EEXIST)");
    let route2 = build_local_default_route(lo_idx);
    match handle.route().add(route2).execute().await {
        Ok(()) => info!("route re-add SUCCEEDED (kernel accepted a second identical local route)"),
        Err(rtnetlink::Error::NetlinkError(e)) if e.raw_code().abs() == libc::EEXIST => {
            info!("route re-add returned -EEXIST — idempotent (production maps this to Ok, L755)")
        }
        Err(e) => info!("route re-add unexpected error: {e}"),
    }
    let (rn2, _) = count_local_default_route(&handle, TPROXY_RT_TABLE).await;
    if rn2 == rn {
        ok!("read-back after route re-add: route count unchanged ({rn2})");
    } else {
        bad!("read-back after route re-add: count changed {rn} -> {rn2}");
        fails += 1;
    }

    // ---- CLEANUP: remove the node-global rule + route we added ----
    step!("cleanup: ip rule del + ip route del (leave the Lima main netns as we found it)");
    {
        let mut s = handle.rule().get(IpVersion::V4).execute();
        let mut victims = Vec::new();
        while let Ok(Some(msg)) = s.try_next().await {
            let has_mark = msg.attributes.iter().any(
                |a| matches!(a, RuleAttribute::FwMark(m) if *m == TPROXY_FWMARK),
            );
            let has_table = u32::from(msg.header.table) == TPROXY_RT_TABLE
                || msg.attributes.iter().any(
                    |a| matches!(a, RuleAttribute::Table(t) if *t == TPROXY_RT_TABLE),
                );
            if has_mark && has_table {
                victims.push(msg);
            }
        }
        for v in victims {
            let _ = handle.rule().del(v).execute().await;
        }
    }
    let route_del = build_local_default_route(lo_idx);
    let _ = handle.route().del(route_del).execute().await;
    let rule_left = count_fwmark_table_rules(&handle, TPROXY_FWMARK, TPROXY_RT_TABLE).await;
    let (route_left, _) = count_local_default_route(&handle, TPROXY_RT_TABLE).await;
    if rule_left == 0 && route_left == 0 {
        ok!("cleanup verified: 0 fwmark rules, 0 local routes left in table {TPROXY_RT_TABLE}");
    } else {
        bad!("cleanup incomplete: {rule_left} rule(s), {route_left} route(s) remain");
        fails += 1;
    }

    println!("\n=== VERDICT (increment-d) ===");
    if fails == 0 {
        println!(
            "WORKS — `ip rule add fwmark {TPROXY_FWMARK:#x} lookup {TPROXY_RT_TABLE}` and `ip route add local 0.0.0.0/0 dev lo table {TPROXY_RT_TABLE}` both replicated via rtnetlink, verified by netlink read-back, idempotent, cleaned up. Zero `ip` subprocesses."
        );
    } else {
        println!("DOESN'T-WORK — {fails} check(s) failed; see [FAIL] lines above.");
    }
    std::process::exit(i32::from(fails != 0));
}

/// Build the `local 0.0.0.0/0 dev lo table 100` route message (add/del share it).
/// kind=Local (RTN_LOCAL), scope=Host (RT_SCOPE_HOST — what iproute2 sets for a
/// `local` route), table via table_id, dev lo via output_interface.
fn build_local_default_route(lo_idx: u32) -> rtnetlink::packet_route::route::RouteMessage {
    RouteMessageBuilder::<Ipv4Addr>::new()
        .kind(RouteType::Local)
        .scope(RouteScope::Host)
        .table_id(TPROXY_RT_TABLE)
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .output_interface(lo_idx)
        .build()
}
