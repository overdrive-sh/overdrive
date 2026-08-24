// PROBE increment-c — subprocess-free-veth-provisioner (GH #233).
//
// Proves: the two per-netns sysctls in veth_provisioner.rs can be driven by
// plain `/proc/sys/**` file I/O instead of the `sysctl -w`/`sysctl -n`
// subprocesses (sysctl_set L2581 / sysctl_read L2688), AND that `/proc/sys/net/**`
// is per-netns — a write from INSIDE the target netns takes there and does NOT
// leak to the host netns. This is the load-bearing constraint for the production
// swap: the in-netns knobs (EnableIpForward L2294, RelaxGlobalRpFilter L2296)
// must be written from inside the alloc's netns (setns), exactly like
// `ip netns exec <ns> sysctl -w` does today.
//
// Mechanism: create a named netns the production way (rtnetlink
// NetworkNamespace::add — pure syscalls), setns a dedicated thread into it,
// write /proc/sys via std::fs, read back; the host (main thread) never leaves
// its netns, so its knobs must stay at baseline.

use std::fs;
use std::os::fd::AsRawFd;

use rtnetlink::NetworkNamespace;

const NETNS: &str = "ovd-ns-c0";
const NETNS_PATH: &str = "/var/run/netns/ovd-ns-c0";

// The two real knobs (veth_provisioner.rs) as /proc/sys paths.
const IP_FORWARD: &str = "/proc/sys/net/ipv4/ip_forward"; // net.ipv4.ip_forward
const RP_FILTER_ALL: &str = "/proc/sys/net/ipv4/conf/all/rp_filter"; // net.ipv4.conf.all.rp_filter

fn read_sysctl(path: &str) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_else(|e| format!("<err: {e}>"))
}
fn write_sysctl(path: &str, val: &str) -> std::io::Result<()> {
    fs::write(path, val)
}

fn main() {
    println!("=== PROBE increment-c: per-netns sysctl via /proc/sys file I/O (subprocess-free) ===");
    println!("kernel: {}", read_sysctl("/proc/sys/kernel/osrelease"));
    println!("knobs: ip_forward (EnableIpForward L2294), conf.all.rp_filter (RelaxGlobalRpFilter L2296)\n");

    // ---- HOST baseline (main thread stays in the host netns throughout) ----
    let host_ipf_0 = read_sysctl(IP_FORWARD);
    let host_rpf_0 = read_sysctl(RP_FILTER_ALL);
    println!(">>> HOST baseline");
    println!("    host net.ipv4.ip_forward            = {host_ipf_0}");
    println!("    host net.ipv4.conf.all.rp_filter    = {host_rpf_0}\n");

    // ---- create the named netns the production way (pure syscalls) ----
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let _ = rt.block_on(NetworkNamespace::del(NETNS.to_string())); // pre-clean
    rt.block_on(NetworkNamespace::add(NETNS.to_string())).expect("netns add");
    println!(">>> created netns {NETNS} (NetworkNamespace::add — no `ip netns` subprocess)\n");

    // ---- write + read the sysctls from INSIDE the netns (setns'd thread) ----
    let report = std::thread::spawn(|| -> Vec<String> {
        let mut log = Vec::new();
        let nsf = fs::File::open(NETNS_PATH).expect("open netns fd");
        if unsafe { libc::setns(nsf.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
            log.push(format!("[FAIL] setns: {}", std::io::Error::last_os_error()));
            return log;
        }
        log.push("[PASS] setns into netns (thread now in ovd-ns-c0)".into());

        // ip_forward: fresh netns default, then production value 1.
        let ns_ipf_default = read_sysctl(IP_FORWARD);
        write_sysctl(IP_FORWARD, "1").expect("write ip_forward");
        let ns_ipf = read_sysctl(IP_FORWARD);
        log.push(format!("NETNS ip_forward: default={ns_ipf_default} -> wrote 1 -> readback={ns_ipf}"));

        // rp_filter(all): sentinel 2 (loose) to force a value distinct from the
        // host, prove it took, then the production value 0.
        let ns_rpf_default = read_sysctl(RP_FILTER_ALL);
        write_sysctl(RP_FILTER_ALL, "2").expect("write rp_filter sentinel");
        let ns_rpf_sentinel = read_sysctl(RP_FILTER_ALL);
        write_sysctl(RP_FILTER_ALL, "0").expect("write rp_filter production");
        let ns_rpf = read_sysctl(RP_FILTER_ALL);
        log.push(format!(
            "NETNS conf.all.rp_filter: default={ns_rpf_default} -> sentinel 2 readback={ns_rpf_sentinel} -> wrote 0 -> readback={ns_rpf}"
        ));
        log.push(format!("__NS_IPF__={ns_ipf}"));
        log.push(format!("__NS_RPF_SENTINEL__={ns_rpf_sentinel}"));
        log.push(format!("__NS_RPF__={ns_rpf}"));
        log
    })
    .join()
    .unwrap();

    println!(">>> IN-NETNS writes (setns'd thread, /proc/sys file I/O)");
    let mut ns_ipf = String::new();
    let mut ns_rpf_sentinel = String::new();
    let mut ns_rpf = String::new();
    for l in &report {
        if let Some(v) = l.strip_prefix("__NS_IPF__=") {
            ns_ipf = v.to_string();
        } else if let Some(v) = l.strip_prefix("__NS_RPF_SENTINEL__=") {
            ns_rpf_sentinel = v.to_string();
        } else if let Some(v) = l.strip_prefix("__NS_RPF__=") {
            ns_rpf = v.to_string();
        } else {
            println!("    {l}");
        }
    }

    // ---- HOST after: must be UNCHANGED (isolation) ----
    let host_ipf_1 = read_sysctl(IP_FORWARD);
    let host_rpf_1 = read_sysctl(RP_FILTER_ALL);
    println!("\n>>> HOST after the in-netns writes (must be unchanged)");
    println!("    host net.ipv4.ip_forward            = {host_ipf_1} (baseline {host_ipf_0})");
    println!("    host net.ipv4.conf.all.rp_filter    = {host_rpf_1} (baseline {host_rpf_0})\n");

    // ---- Isolation table ----
    println!(">>> ISOLATION (same knob, two namespaces, independent values)");
    println!("    knob                          host        netns");
    println!("    net.ipv4.ip_forward           {host_ipf_1:<11} {ns_ipf}   (production write = 1)");
    println!("    net.ipv4.conf.all.rp_filter   {host_rpf_1:<11} {ns_rpf_sentinel} then {ns_rpf}  (sentinel 2, then production 0)\n");

    // ---- Verdict ----
    let writes_took = ns_ipf == "1" && ns_rpf_sentinel == "2" && ns_rpf == "0";
    let host_unchanged = host_ipf_1 == host_ipf_0 && host_rpf_1 == host_rpf_0;
    // Guaranteed value contrast: the sentinel 2 in the netns vs the host rp_filter.
    let distinct_coexist = ns_rpf_sentinel == "2" && host_rpf_1 != "2";

    println!("=== VERDICT ===");
    println!("  /proc/sys writes took inside netns (ip_forward=1, rp_filter 2->0): {}", if writes_took { "PASS" } else { "FAIL" });
    println!("  host knobs unchanged by the in-netns writes (isolation):          {}", if host_unchanged { "PASS" } else { "FAIL" });
    println!("  distinct values coexist per-netns (netns rp_filter=2, host!={}):    {}", host_rpf_1, if distinct_coexist { "PASS" } else { "FAIL" });

    // cleanup
    let _ = rt.block_on(NetworkNamespace::del(NETNS.to_string()));

    if writes_took && host_unchanged && distinct_coexist {
        println!("WORKS — /proc/sys/net/** sysctls are per-netns; file I/O from inside the netns replaces `sysctl -w`, with zero host leakage.");
    } else {
        println!("DOESN'T-WORK — see FAIL lines above.");
    }
}
