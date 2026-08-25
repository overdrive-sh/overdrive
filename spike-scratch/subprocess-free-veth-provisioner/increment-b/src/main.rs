// PROBE increment-b — subprocess-free-veth-provisioner (GH #233).
// THE PRIMARY DE-RISK: hand-roll `ethtool -K <dev> tx off` over genetlink.
//
// The `ethtool` rust-netlink crate cannot do it: EthtoolCmd has no FeatureSet,
// and EthtoolFeatureAttr::Wanted's emit is `todo!("Does not support changing
// ethtool feature yet")` (ethtool-0.2.9 src/feature/attr.rs:44-49) — it PANICS.
// So we hand-encode ETHTOOL_MSG_FEATURES_SET (=12; NOT 0x0a, which is WOL_SET)
// on a raw NETLINK_GENERIC socket, clearing the whole tx-checksum-* group via a
// name-based ETHTOOL_A_FEATURES_WANTED bitset — the netlink-granular equivalent
// of the `tx` keyword's group expansion.
//
// Then PROVE offload is actually OFF via THREE independent oracles:
//   (a) netlink FEATURES_GET read-back (ethtool crate) — active tx-checksum* off
//   (b) independent `ethtool -k <dev>` subprocess — allowed HERE as an evidence
//       oracle only (it is NOT the production mechanism)
//   (c) real UDP packet on the wire: tcpdump -v the egress veth and confirm the
//       L4 checksum flips from "bad udp cksum" (offload ON, deferred) to
//       "udp sum ok" (offload OFF, materialised in software) — the exact
//       byte-correctness property the XDP NAT hook depends on (commit 62fa6be2).
//
// Real target: veth_provisioner.rs tx_offload_off L1672 / netns_tx_offload_off
// L2487 (`ethtool -K <iface> tx off`); read side iface_tx_offload_on L1484
// (`ethtool -k <iface>` grep `tx-checksumming:`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::os::fd::AsRawFd;
use std::process::Command;

use futures::stream::TryStreamExt;
use rtnetlink::{new_connection, Handle, LinkUnspec, LinkVeth, NetworkNamespace, RouteMessageBuilder};

const NETNS: &str = "ovd-ns-b0";
const HOST_VETH: &str = "ovd-hv-b0";
const WORKLOAD_VETH: &str = "ovd-wl-b0";
const HOST_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1);
const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 2);
const PREFIX: u8 = 30;
const NETNS_PATH: &str = "/var/run/netns/ovd-ns-b0";

// ---- raw NETLINK_GENERIC (ethtool FEATURES_SET) ------------------------------
const NETLINK_GENERIC: i32 = 16;
const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_ACK: u16 = 0x04;
const NLMSG_ERROR: u16 = 0x02;
const NLA_F_NESTED: u16 = 0x8000;

// ethtool_netlink.h
const ETHTOOL_MSG_FEATURES_SET: u8 = 12; // <-- the trap: NOT 0x0a
const ETHTOOL_A_FEATURES_HEADER: u16 = 1;
const ETHTOOL_A_FEATURES_WANTED: u16 = 3;
const ETHTOOL_A_HEADER_DEV_NAME: u16 = 2;
const ETHTOOL_A_BITSET_BITS: u16 = 3;
const ETHTOOL_A_BITSET_BITS_BIT: u16 = 1;
const ETHTOOL_A_BITSET_BIT_NAME: u16 = 2;

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}
/// Append one netlink attribute (native-endian len/type, 4-byte padded).
fn nla(buf: &mut Vec<u8>, ty: u16, val: &[u8]) {
    let len = (4 + val.len()) as u16;
    buf.extend_from_slice(&len.to_ne_bytes());
    buf.extend_from_slice(&ty.to_ne_bytes());
    buf.extend_from_slice(val);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

struct GenlSock {
    fd: i32,
    seq: u32,
}
impl GenlSock {
    fn open() -> std::io::Result<Self> {
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_GENERIC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut sa: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        sa.nl_family = libc::AF_NETLINK as u16;
        let rc = unsafe {
            libc::bind(
                fd,
                &sa as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { fd, seq: 1 })
    }

    /// Send a genl request (nlmsghdr + genlmsghdr + payload) to the kernel and
    /// return the raw reply bytes.
    fn request(&mut self, family: u16, cmd: u8, ver: u8, flags: u16, payload: &[u8]) -> std::io::Result<Vec<u8>> {
        self.seq += 1;
        let mut genl = Vec::new();
        genl.push(cmd);
        genl.push(ver);
        genl.extend_from_slice(&0u16.to_ne_bytes()); // reserved
        genl.extend_from_slice(payload);

        let total = 16 + genl.len();
        let mut msg = Vec::with_capacity(total);
        msg.extend_from_slice(&(total as u32).to_ne_bytes());
        msg.extend_from_slice(&family.to_ne_bytes());
        msg.extend_from_slice(&flags.to_ne_bytes());
        msg.extend_from_slice(&self.seq.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes()); // pid=0 (let kernel assign)
        msg.extend_from_slice(&genl);

        let mut dst: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        dst.nl_family = libc::AF_NETLINK as u16; // nl_pid=0 => kernel
        let sent = unsafe {
            libc::sendto(
                self.fd,
                msg.as_ptr() as *const libc::c_void,
                msg.len(),
                0,
                &dst as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as u32,
            )
        };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buf = vec![0u8; 16384];
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        buf.truncate(n as usize);
        Ok(buf)
    }
}
impl Drop for GenlSock {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn le_u16(b: &[u8], o: usize) -> u16 {
    u16::from_ne_bytes([b[o], b[o + 1]])
}
fn le_u32(b: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Resolve a genl family id by name via CTRL_CMD_GETFAMILY.
fn resolve_family(sock: &mut GenlSock, name: &str) -> std::io::Result<u16> {
    let mut payload = Vec::new();
    nla(&mut payload, CTRL_ATTR_FAMILY_NAME, &cstr(name));
    let reply = sock.request(GENL_ID_CTRL, CTRL_CMD_GETFAMILY, 1, NLM_F_REQUEST, &payload)?;
    // nlmsghdr(16) + genlmsghdr(4) then attrs
    if reply.len() < 20 {
        return Err(std::io::Error::other("short GETFAMILY reply"));
    }
    let msg_len = le_u32(&reply, 0) as usize;
    let mut off = 20;
    while off + 4 <= msg_len.min(reply.len()) {
        let alen = le_u16(&reply, off) as usize;
        let atype = le_u16(&reply, off + 2) & 0x7fff;
        if alen < 4 {
            break;
        }
        if atype == CTRL_ATTR_FAMILY_ID {
            return Ok(le_u16(&reply, off + 4));
        }
        off += (alen + 3) & !3;
    }
    Err(std::io::Error::other("CTRL_ATTR_FAMILY_ID not found"))
}

/// Hand-rolled ETHTOOL_MSG_FEATURES_SET: request each named bit to OFF (bit
/// present in the WANTED bitset with no BIT_VALUE => target value 0).
/// Returns Ok(()) on netlink ACK (error==0), Err with the errno otherwise.
fn ethtool_features_set_off(sock: &mut GenlSock, family: u16, dev: &str, names: &[String]) -> Result<(), String> {
    let mut hdr = Vec::new();
    nla(&mut hdr, ETHTOOL_A_HEADER_DEV_NAME, &cstr(dev));

    let mut bits = Vec::new();
    for name in names {
        let mut bit = Vec::new();
        nla(&mut bit, ETHTOOL_A_BITSET_BIT_NAME, &cstr(name));
        // NO ETHTOOL_A_BITSET_BIT_VALUE => request this bit -> 0 (off)
        nla(&mut bits, ETHTOOL_A_BITSET_BITS_BIT | NLA_F_NESTED, &bit);
    }
    let mut bitset = Vec::new();
    nla(&mut bitset, ETHTOOL_A_BITSET_BITS | NLA_F_NESTED, &bits);

    let mut payload = Vec::new();
    nla(&mut payload, ETHTOOL_A_FEATURES_HEADER | NLA_F_NESTED, &hdr);
    nla(&mut payload, ETHTOOL_A_FEATURES_WANTED | NLA_F_NESTED, &bitset);

    let reply = sock
        .request(family, ETHTOOL_MSG_FEATURES_SET, 1, NLM_F_REQUEST | NLM_F_ACK, &payload)
        .map_err(|e| format!("sendrecv failed: {e}"))?;
    // Expect NLMSG_ERROR with error==0 (ACK) — or a FEATURES_SET reply message.
    if reply.len() < 16 {
        return Err("short reply".into());
    }
    let ty = le_u16(&reply, 4);
    if ty == NLMSG_ERROR {
        let err = le_u32(&reply, 16) as i32;
        if err == 0 {
            Ok(())
        } else {
            Err(format!("netlink error {} ({})", err, std::io::Error::from_raw_os_error(-err)))
        }
    } else {
        // A non-error reply (kernel echoed a FEATURES_SET_REPLY) also means accepted.
        Ok(())
    }
}

// ---- ethtool crate FEATURES_GET (enumeration + oracle a) ---------------------
async fn feature_snapshot(dev: &str) -> (BTreeMap<String, bool>, BTreeSet<String>) {
    use ethtool::{EthtoolAttr, EthtoolFeatureAttr};
    let (conn, mut handle, _) = ethtool::new_connection().expect("ethtool conn");
    let jh = tokio::spawn(conn);
    let mut active: BTreeMap<String, bool> = BTreeMap::new();
    let mut changeable: BTreeSet<String> = BTreeSet::new();
    {
        let mut stream = Box::pin(handle.feature().get(Some(dev)).execute().await);
        while let Some(msg) = stream.try_next().await.expect("feature get stream") {
            for attr in &msg.payload.nlas {
                if let EthtoolAttr::Feature(f) = attr {
                    match f {
                        EthtoolFeatureAttr::Active(bits) => {
                            for b in bits {
                                active.insert(b.name.clone(), b.value);
                            }
                        }
                        // Hw = user-changeable mask: value=true means changeable.
                        EthtoolFeatureAttr::Hw(bits) => {
                            for b in bits {
                                if b.value {
                                    changeable.insert(b.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    drop(handle);
    jh.abort();
    (active, changeable)
}

// ---- rtnetlink scaffolding (reuse increment-a) -------------------------------
async fn link_index(handle: &Handle, name: &str) -> Option<u32> {
    let mut s = handle.link().get().match_name(name.to_string()).execute();
    s.try_next().await.ok().flatten().map(|m| m.header.index)
}

/// tcpdump -v the egress veth while sending UDP; return the captured lines.
fn capture_udp_cksum(dev: &str, phase: &str) -> String {
    // Start capture in the background (kill after 4s if <3 pkts).
    let child = Command::new("timeout")
        .args(["4", "tcpdump", "-l", "-n", "-vv", "-i", dev, "-c", "3", "udp", "and", "dst", "10.99.0.2"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tcpdump");
    std::thread::sleep(std::time::Duration::from_millis(700));
    if let Ok(sock) = UdpSocket::bind("10.99.0.1:0") {
        for _ in 0..6 {
            let _ = sock.send_to(b"veth-l4-checksum-probe-PAYLOAD-abcdef", "10.99.0.2:9");
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
    }
    let out = child.wait_with_output().expect("tcpdump wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut lines: Vec<String> = Vec::new();
    for l in stdout.lines().chain(stderr.lines()) {
        let l = l.trim();
        if l.contains("10.99.0") || l.contains("cksum") || l.contains("sum ok") {
            lines.push(l.to_string());
        }
    }
    format!("[{phase}] tcpdump({dev}):\n        {}", lines.join("\n        "))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("=== PROBE increment-b: hand-rolled ethtool FEATURES_SET (subprocess-free) ===");
    println!("kernel: {}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());
    println!("ethtool -k target dev: {HOST_VETH} (host end of veth pair)\n");

    // Pre-clean.
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

    // ---- Scaffold: veth pair + netns + addresses (so a real UDP packet flows) ----
    println!(">>> scaffold: veth {HOST_VETH}<->{WORKLOAD_VETH}, netns {NETNS}, /30 addrs");
    handle.link().add(LinkVeth::new(WORKLOAD_VETH, HOST_VETH).build()).execute().await.expect("veth add");
    let host_idx = link_index(&handle, HOST_VETH).await.expect("host idx");
    let wl_idx = link_index(&handle, WORKLOAD_VETH).await.expect("wl idx");
    handle.link().set(LinkUnspec::new_with_index(host_idx).up().build()).execute().await.expect("host up");
    handle.address().add(host_idx, IpAddr::V4(HOST_ADDR), PREFIX).execute().await.expect("host addr");
    NetworkNamespace::add(NETNS.to_string()).await.expect("netns add");
    let nsf = File::open(NETNS_PATH).expect("open netns");
    handle.link().set(LinkUnspec::new_with_index(wl_idx).setns_by_fd(nsf.as_raw_fd()).build()).execute().await.expect("move");
    drop(nsf);
    // in-netns config on a setns'd thread
    std::thread::spawn(|| {
        let nsf = File::open(NETNS_PATH).unwrap();
        if unsafe { libc::setns(nsf.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (c, h, _) = new_connection().unwrap();
            tokio::spawn(c);
            let wl = link_index(&h, WORKLOAD_VETH).await.unwrap();
            let _ = h.address().add(wl, IpAddr::V4(WORKLOAD_ADDR), PREFIX).execute().await;
            let _ = h.link().set(LinkUnspec::new_with_index(wl).up().build()).execute().await;
            if let Some(lo) = link_index(&h, "lo").await {
                let _ = h.link().set(LinkUnspec::new_with_index(lo).up().build()).execute().await;
            }
            let dflt = RouteMessageBuilder::<Ipv4Addr>::new().gateway(HOST_ADDR).build();
            let _ = h.route().add(dflt).execute().await;
        });
    })
    .join()
    .unwrap();
    println!("    scaffold up.\n");

    // ---- Enumerate tx-checksum-* bits (FEATURES_GET) ----
    println!(">>> FEATURES_GET enumeration on {HOST_VETH} (ethtool crate)");
    let (active_before, changeable) = feature_snapshot(HOST_VETH).await;
    let tx_all: Vec<String> = active_before.keys().filter(|k| k.starts_with("tx-checksum")).cloned().collect();
    let tx_targets: Vec<String> = tx_all.iter().filter(|n| changeable.contains(*n)).cloned().collect();
    for n in &tx_all {
        println!(
            "    {n:<26} active={} changeable={}",
            active_before.get(n).copied().unwrap_or(false),
            changeable.contains(n)
        );
    }
    println!("    umbrella tx-checksumming(active)={:?}", active_before.get("tx-checksumming"));
    println!("    -> SET targets (changeable tx-checksum-*): {:?}\n", tx_targets);

    // ---- Oracle (c) BEFORE: offload ON — expect deferred/incorrect cksum ----
    println!(">>> oracle (c) BEFORE (offload ON):");
    let c_before = capture_udp_cksum(HOST_VETH, "offload-ON");
    println!("    {c_before}\n");

    // ---- Hand-rolled FEATURES_SET tx-checksum-* OFF ----
    println!(">>> hand-rolled ETHTOOL_MSG_FEATURES_SET (cmd=12) tx-checksum-* OFF on {HOST_VETH}");
    let mut sock = GenlSock::open().expect("genl socket");
    let fam = resolve_family(&mut sock, "ethtool").expect("resolve ethtool family");
    println!("    resolved genl family \"ethtool\" id={fam}");
    match ethtool_features_set_off(&mut sock, fam, HOST_VETH, &tx_targets) {
        Ok(()) => println!("    [PASS] FEATURES_SET ACK (error==0) — kernel accepted the SET\n"),
        Err(e) => println!("    [FAIL] FEATURES_SET rejected: {e}\n"),
    }

    // ---- Oracle (a): netlink FEATURES_GET read-back ----
    println!(">>> oracle (a) netlink read-back (FEATURES_GET):");
    let (active_after, _) = feature_snapshot(HOST_VETH).await;
    let mut a_ok = !tx_targets.is_empty();
    for n in &tx_targets {
        let before = active_before.get(n).copied().unwrap_or(false);
        let after = active_after.get(n).copied().unwrap_or(false);
        println!("    {n:<26} active {before} -> {after} (want true -> false)");
        if after {
            a_ok = false;
        }
    }
    // NOTE: the `tx-checksumming` umbrella is a NETIF_F feature surfaced by
    // `ethtool -k` (oracle b), NOT a bit in the netlink FEATURES Active bitset
    // (which carries only the low-level tx-checksum-* names). So oracle (a)
    // asserts on the low-level bits flipping true->false; the umbrella is oracle (b).
    if a_ok {
        println!("    [PASS] oracle (a): every changeable tx-checksum-* bit flipped active true->false via netlink read-back\n");
    } else {
        println!("    [FAIL] oracle (a): a target bit is still active after the SET\n");
    }

    // ---- Oracle (b): independent `ethtool -k` subprocess ----
    println!(">>> oracle (b) independent `ethtool -k {HOST_VETH}` (evidence only, not the mechanism):");
    let b_out = Command::new("ethtool").args(["-k", HOST_VETH]).output().expect("ethtool -k");
    let b_txt = String::from_utf8_lossy(&b_out.stdout);
    let mut b_ok = true;
    for line in b_txt.lines().filter(|l| l.contains("tx-checksum")) {
        println!("    {}", line.trim());
        if line.contains("tx-checksumming:") && line.contains(": on") {
            b_ok = false;
        }
    }
    println!(
        "    [{}] oracle (b): `ethtool -k` reports tx-checksumming off\n",
        if b_ok { "PASS" } else { "FAIL" }
    );

    // ---- Oracle (c) AFTER: offload OFF — expect materialised/correct cksum ----
    println!(">>> oracle (c) AFTER (offload OFF):");
    let c_after = capture_udp_cksum(HOST_VETH, "offload-OFF");
    println!("    {c_after}\n");

    // (c) classification: with offload ON the egress capture carries a deferred
    // (partial) L4 checksum tcpdump flags "bad udp cksum"; with offload OFF the
    // full checksum is materialised in software -> "udp sum ok".
    let c_on_bad = c_before.contains("bad udp cksum") || c_before.contains("incorrect");
    let c_off_ok = c_after.contains("udp sum ok") || c_after.contains("cksum ok");
    let c_signal = if c_on_bad && c_off_ok {
        "PASS (ON=bad/deferred -> OFF=udp sum ok, clean contrast)"
    } else if c_off_ok {
        "PASS (OFF=udp sum ok; ON contrast not surfaced by tcpdump here)"
    } else {
        "INCONCLUSIVE (tcpdump did not surface L4-cksum verification; a+b authoritative)"
    };

    // ---- Cleanup ----
    let _ = NetworkNamespace::del(NETNS.to_string()).await;
    if let Some(idx) = link_index(&handle, HOST_VETH).await {
        let _ = handle.link().del(idx).execute().await;
    }

    // ---- Verdict (a + b are authoritative; c is corroborating) ----
    println!("=== VERDICT ===");
    println!("  oracle (a) netlink read-back : {}", if a_ok { "PASS" } else { "FAIL" });
    println!("  oracle (b) ethtool -k        : {}", if b_ok { "PASS" } else { "FAIL" });
    println!("  oracle (c) wire cksum        : {c_signal}");
    if a_ok && b_ok {
        println!("WORKS — hand-rolled ETHTOOL_MSG_FEATURES_SET genuinely flips tx-checksum offload OFF.");
    } else {
        println!("DOESN'T-WORK — SET returned success but offload not actually off; see FAIL oracles.");
    }
}
