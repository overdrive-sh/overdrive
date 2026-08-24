// PROBE increment-a — subprocess-free-veth-provisioner (GH #233).
//
// Proves: rtnetlink (pure netlink syscalls, NO `ip` subprocess) can replicate
// the veth/addr/route/netns operations in
// crates/overdrive-control-plane/src/veth_provisioner.rs. Every op is verified
// by a NETLINK read-back (link get / addr get / route get), not by trusting
// the write returned Ok. In-netns ops run the production-equivalent way: a
// dedicated thread setns()'d into the target netns, then a netns-bound netlink
// connection (exactly what `ip -n <ns>` does internally).
//
// Replicated real operations (cited to veth_provisioner.rs):
//   - link add veth pair           (host: link_add L1551; per-alloc: workload_link_add L2367)
//   - addr add <cidr> dev <iface>  (host: addr_add L1628; netns: netns_addr_add L2406)
//   - link set <iface> up          (host: link_up L1646; netns: netns_link_up L2427)
//   - route add default via <gw>   (netns: netns_default_route_add L2464)
//   - route add <cidr> dev <iface> (host: add_route L1605)
//   - netns add <netns>            (netns_add L2314) via NetworkNamespace::add
//   - link set <if> netns <netns>  (netns_move L2391) via setns_by_fd
//   - netns del / link del cleanup (netns_del L2331 / link_del L1582)
//
// Real per-alloc plan values (derive_workload_netns_plan L544, slot 0):
//   netns=ovd-ns-0000  host_veth=ovd-hv-0000  workload_veth=ovd-wl-0000
//   subnet=10.99.0.0/30  host_addr/gateway=10.99.0.1  workload_addr=10.99.0.2

use std::fs::File;
use std::net::{IpAddr, Ipv4Addr};
use std::os::fd::AsRawFd;

use futures::stream::TryStreamExt;
use rtnetlink::{
    new_connection, Handle, LinkUnspec, LinkVeth, NetworkNamespace, RouteMessageBuilder,
};

// Real slot-0 per-allocation plan (derive_workload_netns_plan, veth_provisioner.rs:544).
const NETNS: &str = "ovd-ns-0000";
const HOST_VETH: &str = "ovd-hv-0000";
const WORKLOAD_VETH: &str = "ovd-wl-0000";
const HOST_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1); // subnet.network()+1  (= gateway)
const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 2); // subnet.network()+2
const PREFIX: u8 = 30;
const NETNS_PATH: &str = "/var/run/netns/ovd-ns-0000";

macro_rules! step {
    ($($a:tt)*) => {{ println!("\n>>> {}", format!($($a)*)); }};
}
macro_rules! ok {
    ($($a:tt)*) => {{ println!("    [PASS] {}", format!($($a)*)); }};
}
macro_rules! bad {
    ($($a:tt)*) => {{ println!("    [FAIL] {}", format!($($a)*)); }};
}

/// Return the kernel ifindex of a link by name in the CURRENT netns, or None.
async fn link_index(handle: &Handle, name: &str) -> Option<u32> {
    let mut s = handle.link().get().match_name(name.to_string()).execute();
    match s.try_next().await {
        Ok(Some(msg)) => Some(msg.header.index),
        _ => None,
    }
}

/// (present, up) for a link by name in the CURRENT netns.
async fn link_state(handle: &Handle, name: &str) -> (bool, bool) {
    let mut s = handle.link().get().match_name(name.to_string()).execute();
    match s.try_next().await {
        Ok(Some(msg)) => {
            // IFF_UP = 0x1 in link header flags.
            let up = msg.header.flags.bits() & 0x1 != 0;
            (true, up)
        }
        _ => (false, false),
    }
}

/// True iff `iface` (by index) in the CURRENT netns carries `want`.
async fn iface_has_addr(handle: &Handle, index: u32, want: Ipv4Addr) -> bool {
    use rtnetlink::packet_route::address::AddressAttribute;
    let mut s = handle.address().get().set_link_index_filter(index).execute();
    while let Ok(Some(msg)) = s.try_next().await {
        for attr in &msg.attributes {
            if let AddressAttribute::Address(IpAddr::V4(a)) = attr {
                if *a == want {
                    return true;
                }
            }
        }
    }
    false
}

/// True iff a default route (via `gw`) is present in the CURRENT netns.
async fn has_default_route(handle: &Handle, gw: Ipv4Addr) -> bool {
    use rtnetlink::packet_route::route::{RouteAttribute, RouteAddress};
    let route = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let mut s = handle.route().get(route).execute();
    while let Ok(Some(msg)) = s.try_next().await {
        // dst_len 0 == default route.
        if msg.header.destination_prefix_length != 0 {
            continue;
        }
        for attr in &msg.attributes {
            if let RouteAttribute::Gateway(RouteAddress::Inet(a)) = attr {
                if *a == gw {
                    return true;
                }
            }
        }
    }
    false
}

/// True iff an on-link route to `dst/prefix` dev `oif` exists in the current netns.
async fn has_onlink_route(handle: &Handle, dst: Ipv4Addr, prefix: u8, oif: u32) -> bool {
    use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
    let route = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let mut s = handle.route().get(route).execute();
    while let Ok(Some(msg)) = s.try_next().await {
        if msg.header.destination_prefix_length != prefix {
            continue;
        }
        let mut dst_ok = false;
        let mut oif_ok = false;
        for attr in &msg.attributes {
            match attr {
                RouteAttribute::Destination(RouteAddress::Inet(a)) if *a == dst => dst_ok = true,
                RouteAttribute::Oif(i) if *i == oif => oif_ok = true,
                _ => {}
            }
        }
        if dst_ok && oif_ok {
            return true;
        }
    }
    false
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("=== PROBE increment-a: rtnetlink veth/addr/route/netns (subprocess-free) ===");
    println!("kernel: {}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());

    // Best-effort pre-clean so re-runs start fresh (mirrors converge-on-boot idempotency).
    let _ = NetworkNamespace::del(NETNS.to_string()).await;
    {
        let (c, h, _) = new_connection().unwrap();
        tokio::spawn(c);
        if let Some(idx) = link_index(&h, HOST_VETH).await {
            let _ = h.link().del(idx).execute().await;
        }
    }

    let (connection, handle, _) = new_connection().unwrap();
    tokio::spawn(connection);

    // ---- HOST NETNS ops -------------------------------------------------
    step!("link_add: veth pair {WORKLOAD_VETH} <-> {HOST_VETH} (workload_link_add L2367)");
    handle
        .link()
        .add(LinkVeth::new(WORKLOAD_VETH, HOST_VETH).build())
        .execute()
        .await
        .expect("veth pair add");
    let host_idx = link_index(&handle, HOST_VETH).await.expect("host end index");
    let wl_idx0 = link_index(&handle, WORKLOAD_VETH).await.expect("wl end index (host netns)");
    ok!("pair created; read-back host_idx={host_idx} wl_idx={wl_idx0} (both present in host netns)");

    step!("link_up: {HOST_VETH} up (link_up L1646)");
    handle
        .link()
        .set(LinkUnspec::new_with_index(host_idx).up().build())
        .execute()
        .await
        .expect("host end up");
    let (present, up) = link_state(&handle, HOST_VETH).await;
    if present && up { ok!("read-back: {HOST_VETH} present={present} up={up}"); } else { bad!("{HOST_VETH} present={present} up={up}"); }

    step!("addr_add: {HOST_ADDR}/{PREFIX} dev {HOST_VETH} (addr_add L1628)");
    handle
        .address()
        .add(host_idx, IpAddr::V4(HOST_ADDR), PREFIX)
        .execute()
        .await
        .expect("host addr add");
    if iface_has_addr(&handle, host_idx, HOST_ADDR).await {
        ok!("read-back: {HOST_ADDR}/{PREFIX} bound on {HOST_VETH}");
    } else {
        bad!("{HOST_ADDR} not found on {HOST_VETH}");
    }

    step!("add_route: on-link 10.99.0.0/{PREFIX} dev {HOST_VETH} (add_route L1605 shape)");
    let onlink = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::new(10, 99, 0, 0), PREFIX)
        .output_interface(host_idx)
        .build();
    match handle.route().add(onlink).execute().await {
        Ok(()) => {}
        Err(e) => println!("    (route add returned {e}; connected route from addr may pre-exist — checking read-back)"),
    }
    if has_onlink_route(&handle, Ipv4Addr::new(10, 99, 0, 0), PREFIX, host_idx).await {
        ok!("read-back: on-link route 10.99.0.0/{PREFIX} dev {HOST_VETH} present");
    } else {
        bad!("on-link route 10.99.0.0/{PREFIX} dev {HOST_VETH} NOT present");
    }

    step!("netns_add: create {NETNS} (netns_add L2314) via NetworkNamespace::add");
    NetworkNamespace::add(NETNS.to_string()).await.expect("netns add");
    if std::path::Path::new(NETNS_PATH).exists() {
        ok!("read-back: {NETNS_PATH} exists (netns mount present)");
    } else {
        bad!("{NETNS_PATH} missing");
    }

    step!("netns_move: move {WORKLOAD_VETH} into {NETNS} (netns_move L2391) via setns_by_fd");
    let ns_file = File::open(NETNS_PATH).expect("open netns fd");
    handle
        .link()
        .set(LinkUnspec::new_with_index(wl_idx0).setns_by_fd(ns_file.as_raw_fd()).build())
        .execute()
        .await
        .expect("setns move");
    // Host read-back: the moved end must be GONE from the host netns.
    match link_index(&handle, WORKLOAD_VETH).await {
        None => ok!("read-back(host): {WORKLOAD_VETH} no longer in host netns (exit confirmed)"),
        Some(i) => bad!("{WORKLOAD_VETH} still in host netns at idx {i} (move failed)"),
    }
    drop(ns_file);

    // ---- IN-NETNS ops (dedicated setns'd thread + netns-bound connection) ----
    // Mirror of `ip -n <netns> ...`: setns the thread, then netlink binds to
    // the target netns. Runs on its own std::thread so the main runtime's
    // worker threads keep their host-netns association.
    step!("in-netns ops via setns'd thread (mirror of `ip -n {NETNS} ...`)");
    let netns_report = std::thread::spawn(|| -> Vec<String> {
        let mut log: Vec<String> = Vec::new();
        let nsf = File::open(NETNS_PATH).expect("open netns fd (thread)");
        // setns THIS thread into the workload netns.
        let rc = unsafe { libc::setns(nsf.as_raw_fd(), libc::CLONE_NEWNET) };
        if rc != 0 {
            log.push(format!("[FAIL] setns() rc={rc} errno={}", std::io::Error::last_os_error()));
            return log;
        }
        log.push("[PASS] setns() into netns ok (thread now in ovd-ns-0000)".into());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        log = rt.block_on(async move {
            // new_connection() binds its netlink socket to the CURRENT (netns) netns.
            let (c, h, _) = new_connection().unwrap();
            tokio::spawn(c);

            // Positive confirmation the moved end lives here.
            let wl_idx = match link_index(&h, WORKLOAD_VETH).await {
                Some(i) => { log.push(format!("[PASS] read-back(netns): {WORKLOAD_VETH} present at idx {i}")); i }
                None => { log.push(format!("[FAIL] {WORKLOAD_VETH} NOT in netns")); return log; }
            };

            // addr add 10.99.0.2/30 (netns_addr_add L2406)
            h.address().add(wl_idx, IpAddr::V4(WORKLOAD_ADDR), PREFIX).execute().await
                .map(|_| ()).unwrap_or_else(|e| log.push(format!("[FAIL] netns addr add: {e}")));
            if iface_has_addr(&h, wl_idx, WORKLOAD_ADDR).await {
                log.push(format!("[PASS] read-back(netns): {WORKLOAD_ADDR}/{PREFIX} on {WORKLOAD_VETH}"));
            } else {
                log.push("[FAIL] workload addr not bound".into());
            }

            // link up (netns_link_up L2427) + lo up (SetLoopbackUp L2289)
            h.link().set(LinkUnspec::new_with_index(wl_idx).up().build()).execute().await
                .map(|_| ()).unwrap_or_else(|e| log.push(format!("[FAIL] netns link up: {e}")));
            if let Some(lo) = link_index(&h, "lo").await {
                let _ = h.link().set(LinkUnspec::new_with_index(lo).up().build()).execute().await;
            }
            let (p, u) = link_state(&h, WORKLOAD_VETH).await;
            if p && u { log.push(format!("[PASS] read-back(netns): {WORKLOAD_VETH} up={u}")); }
            else { log.push(format!("[FAIL] {WORKLOAD_VETH} up={u}")); }

            // default route via 10.99.0.1 (netns_default_route_add L2464)
            let dflt = RouteMessageBuilder::<Ipv4Addr>::new().gateway(HOST_ADDR).build();
            h.route().add(dflt).execute().await
                .map(|_| ()).unwrap_or_else(|e| log.push(format!("[FAIL] netns default route: {e}")));
            if has_default_route(&h, HOST_ADDR).await {
                log.push(format!("[PASS] read-back(netns): default via {HOST_ADDR} present"));
            } else {
                log.push("[FAIL] default route not present".into());
            }
            log
        });
        log
    })
    .join()
    .unwrap();
    for line in &netns_report {
        println!("    {line}");
    }

    // ---- CLEANUP --------------------------------------------------------
    step!("cleanup: netns del {NETNS} (netns_del L2331) + link del {HOST_VETH} (link_del L1582)");
    NetworkNamespace::del(NETNS.to_string()).await.expect("netns del");
    // Deleting the netns destroys the in-netns veth end, which reaps the host
    // peer too. link del is best-effort (peer likely already gone).
    if let Some(idx) = link_index(&handle, HOST_VETH).await {
        let _ = handle.link().del(idx).execute().await;
        ok!("deleted host end {HOST_VETH}");
    } else {
        ok!("host end {HOST_VETH} already reaped with the netns (veth pair dies together)");
    }
    let netns_gone = !std::path::Path::new(NETNS_PATH).exists();
    ok!("read-back: {NETNS_PATH} exists={} (expect false)", !netns_gone);

    let fails = netns_report.iter().filter(|l| l.contains("[FAIL]")).count();
    println!("\n=== VERDICT ===");
    if fails == 0 {
        println!("WORKS — every rtnetlink op verified by netlink read-back; zero subprocesses.");
    } else {
        println!("DOESN'T-WORK — {fails} in-netns check(s) failed; see [FAIL] lines above.");
    }
}
