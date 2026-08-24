// PROBE increment-e — subprocess-free-veth-provisioner (GH #233), mtls_intercept.rs scope.
//
// PRIMARY de-risk. Proves the nft TPROXY chain+rule of
// crates/overdrive-worker/src/mtls_intercept.rs can be installed via pure-Rust
// netlink (NO `nft` subprocess) AND that the installed rule ACTUALLY DIVERTS a
// real TCP connection to an IP_TRANSPARENT listener (not merely "netlink ACKed").
//
// KEY FINDING (drove the shape of this probe): rustables 0.8.8 has NO typed
// `tproxy` expression (`grep -rli tproxy` on its src is empty) AND no PUBLIC
// escape hatch to inject a raw one — its `nlmsg` module (carrying the
// `NfNetlinkDeserializable` trait that constructs `ExpressionRaw`) is
// `pub(crate)`, and `ExpressionRaw`'s field is private. So the tproxy verb
// CANNOT go through rustables. We therefore HAND-ROLL the tproxy rule over a raw
// NETLINK_NETFILTER socket (nfnetlink batch: NEWRULE with payload/cmp/immediate/
// tproxy/meta/verdict expressions), modelled on kernel net/netfilter/nft_tproxy.c
// + the nftables netlink spec. rustables still does table+chain+exemption and the
// structural handle read-back / by-handle delete (the parts it CAN do).
//
// Replicated REAL operations (cited to mtls_intercept.rs / overdrive_core::dataplane):
//   - nft add table ip overdrive-mtls                                   L658  (rustables)
//   - nft add chain ip … prerouting { type filter hook prerouting
//                                     priority mangle; policy accept; }  L659-675 (rustables)
//   - nft insert rule … meta mark <MTLS_LEG_S_DIAL_MARK> accept          L682-692 (rustables)
//   - nft add rule ip … prerouting ip daddr <vip> tcp dport <vport>
//        tproxy to 127.0.0.1:<agent_port> meta mark set 0x1 accept       L303-323 (HAND-ROLLED)
//   - handle recovery (STRUCTURAL via list_rules_for_chain vs the
//        production `# handle N` text parse)                             L866-924 (rustables)
//   - nft delete rule … handle <N>                                       L615/L1119 (rustables)
//   - ip rule add fwmark 0x1 lookup 100 ; ip route add local 0.0.0.0/0
//        dev lo table 100  (TPROXY plumbing; proven increment-d)         L648-654,L741 (rtnetlink)
//
// REAL constant VALUES (verbatim):
//   NFT_TABLE="overdrive-mtls"(L62) NFT_CHAIN="prerouting"(L67)
//   TPROXY_FWMARK=0x1(L88) TPROXY_RT_TABLE=100(L93)
//   MTLS_LEG_S_DIAL_MARK=0x2 (overdrive-core/src/dataplane/mtls_mark.rs:21)
//
// PROOF (make-or-break): a netns client connects to <vip>:<vport>; the SYN
// ingresses the host veth, hits prerouting, and the tproxy rule diverts it to the
// 127.0.0.1:<agent_port> IP_TRANSPARENT listener in the main netns. Success =
// accept fires AND the accepted socket's getsockname (local_addr) == <vip>:<vport>
// (orig-dst preserved — a FOREIGN-dst connection was diverted) AND a byte-distinct
// REQUEST/RESPONSE round-trips.

use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::mpsc;
use std::time::Duration;

use futures::stream::TryStreamExt;
use rtnetlink::packet_route::route::{RouteScope, RouteType};
use rtnetlink::packet_route::rule::{RuleAction, RuleAttribute};
use rtnetlink::{
    new_connection, Handle, IpVersion, LinkUnspec, LinkVeth, NetworkNamespace, RouteMessageBuilder,
};

use rustables::expr::{Cmp, CmpOp, Meta, MetaType};
use rustables::{
    list_rules_for_chain, Batch, Chain, ChainPolicy, ChainType, Hook, HookClass, MsgType,
    ProtocolFamily, Rule, Table,
};

// ---- REAL constants (verbatim from mtls_intercept.rs / dataplane) ----
const NFT_TABLE: &str = "overdrive-mtls-spike-e"; // real base "overdrive-mtls" + spike suffix
const NFT_CHAIN: &str = "prerouting"; // NFT_CHAIN L67
const TPROXY_FWMARK: u32 = 0x1; // TPROXY_FWMARK L88
const TPROXY_RT_TABLE: u32 = 100; // TPROXY_RT_TABLE L93
const MTLS_LEG_S_DIAL_MARK: u32 = 0x2; // dataplane::MTLS_LEG_S_DIAL_MARK

// ---- probe topology ----
const NETNS: &str = "ovd-spk-e";
const NETNS_PATH: &str = "/var/run/netns/ovd-spk-e";
const HOST_VETH: &str = "spk-hv-e";
const WL_VETH: &str = "spk-wl-e";
const HOST_ADDR: Ipv4Addr = Ipv4Addr::new(10, 98, 0, 1);
const WL_ADDR: Ipv4Addr = Ipv4Addr::new(10, 98, 0, 2);
const PREFIX: u8 = 30;
const VIP: Ipv4Addr = Ipv4Addr::new(10, 200, 0, 1); // foreign dst the client dials
const VPORT: u16 = 9999;
const AGENT_PORT: u16 = 47001; // 127.0.0.1:AGENT_PORT transparent listener (leg-C stand-in)

const REQUEST: &[u8] = b"SPIKE-E-REQ-7f3a\n"; // byte-distinct litmus (client -> server)
const RESPONSE: &[u8] = b"SPIKE-E-RESP-9c21\n"; // byte-distinct litmus (server -> client)

macro_rules! step { ($($a:tt)*) => {{ println!("\n>>> {}", format!($($a)*)); }}; }
macro_rules! ok    { ($($a:tt)*) => {{ println!("    [PASS] {}", format!($($a)*)); }}; }
macro_rules! bad   { ($($a:tt)*) => {{ println!("    [FAIL] {}", format!($($a)*)); }}; }
macro_rules! info  { ($($a:tt)*) => {{ println!("    [INFO] {}", format!($($a)*)); }}; }

// ===================================================================
// Hand-rolled nftables netlink for the tproxy rule (the part rustables can't do).
// All nft integer attributes are big-endian on the wire (kernel `nla_get_be32`);
// netlink message/attr headers (len,type) are host byte order.
// ===================================================================
mod nft_raw {
    use std::io::Error;
    use std::net::Ipv4Addr;

    // nfnetlink subsystem + batch control
    const NFNL_SUBSYS_NFTABLES: u16 = 10;
    const NFNL_MSG_BATCH_BEGIN: u16 = 16;
    const NFNL_MSG_BATCH_END: u16 = 17;
    // nf_tables message ops
    const NFT_MSG_NEWRULE: u16 = 6;
    // netlink flags
    const NLM_F_REQUEST: u16 = 0x001;
    const NLM_F_ACK: u16 = 0x004;
    const NLM_F_CREATE: u16 = 0x400;
    const NLM_F_APPEND: u16 = 0x800;
    // nlattr nested flag
    const NLA_F_NESTED: u16 = 0x8000;
    // families / versions
    const NFPROTO_IPV4: u32 = 2;
    const AF_UNSPEC: u8 = 0;
    // rule attrs
    const NFTA_RULE_TABLE: u16 = 1;
    const NFTA_RULE_CHAIN: u16 = 2;
    const NFTA_RULE_EXPRESSIONS: u16 = 4;
    // list + expr framing
    const NFTA_LIST_ELEM: u16 = 1;
    const NFTA_EXPR_NAME: u16 = 1;
    const NFTA_EXPR_DATA: u16 = 2;
    // data
    const NFTA_DATA_VALUE: u16 = 1;
    const NFTA_DATA_VERDICT: u16 = 2;
    const NFTA_VERDICT_CODE: u16 = 1;
    // payload
    const NFTA_PAYLOAD_DREG: u16 = 1;
    const NFTA_PAYLOAD_BASE: u16 = 2;
    const NFTA_PAYLOAD_OFFSET: u16 = 3;
    const NFTA_PAYLOAD_LEN: u16 = 4;
    const NFT_PAYLOAD_NETWORK_HEADER: u32 = 1;
    const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;
    // cmp
    const NFTA_CMP_SREG: u16 = 1;
    const NFTA_CMP_OP: u16 = 2;
    const NFTA_CMP_DATA: u16 = 3;
    const NFT_CMP_EQ: u32 = 0;
    // immediate
    const NFTA_IMMEDIATE_DREG: u16 = 1;
    const NFTA_IMMEDIATE_DATA: u16 = 2;
    // meta
    const NFTA_META_DREG: u16 = 1;
    const NFTA_META_KEY: u16 = 2;
    const NFTA_META_SREG: u16 = 3;
    const NFT_META_L4PROTO: u32 = 16;
    const NFT_META_MARK: u32 = 3;
    // tproxy (nf_tables.h: FAMILY=1, REG_ADDR=2, REG_PORT=3)
    const NFTA_TPROXY_FAMILY: u16 = 1;
    const NFTA_TPROXY_REG_ADDR: u16 = 2;
    const NFTA_TPROXY_REG_PORT: u16 = 3;
    // registers
    const NFT_REG_VERDICT: u32 = 0;
    const NFT_REG_1: u32 = 1;
    const NFT_REG_2: u32 = 2;
    const NFT_REG_3: u32 = 3;
    // verdict
    const NF_ACCEPT: u32 = 1;
    const IPPROTO_TCP: u8 = 6;

    fn pad4(v: &mut Vec<u8>) {
        while v.len() % 4 != 0 {
            v.push(0);
        }
    }
    /// Append one nlattr [u16 len][u16 type][payload][pad4].
    fn attr(buf: &mut Vec<u8>, typ: u16, payload: &[u8]) {
        let len = 4 + payload.len();
        buf.extend_from_slice(&(len as u16).to_ne_bytes());
        buf.extend_from_slice(&typ.to_ne_bytes());
        buf.extend_from_slice(payload);
        pad4(buf);
    }
    fn attr_be32(buf: &mut Vec<u8>, typ: u16, val: u32) {
        attr(buf, typ, &val.to_be_bytes());
    }
    /// Wrap `inner` as one nested attr of `typ`, returned as fresh bytes.
    fn nested(typ: u16, inner: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        attr(&mut v, typ | NLA_F_NESTED, inner);
        v
    }
    /// One expression as a NFTA_LIST_ELEM: { NFTA_EXPR_NAME, NFTA_EXPR_DATA(nested) }.
    fn expr(name: &str, data: &[u8]) -> Vec<u8> {
        let mut inner = Vec::new();
        let mut namez = name.as_bytes().to_vec();
        namez.push(0);
        attr(&mut inner, NFTA_EXPR_NAME, &namez);
        let mut data_attr = Vec::new();
        attr(&mut data_attr, NFTA_EXPR_DATA | NLA_F_NESTED, data);
        inner.extend_from_slice(&data_attr);
        nested(NFTA_LIST_ELEM, &inner)
    }
    fn data_value(bytes: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        attr(&mut d, NFTA_DATA_VALUE, bytes);
        d
    }

    fn e_payload(base: u32, offset: u32, len: u32, dreg: u32) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_PAYLOAD_DREG, dreg);
        attr_be32(&mut d, NFTA_PAYLOAD_BASE, base);
        attr_be32(&mut d, NFTA_PAYLOAD_OFFSET, offset);
        attr_be32(&mut d, NFTA_PAYLOAD_LEN, len);
        expr("payload", &d)
    }
    fn e_cmp_eq(sreg: u32, value: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_CMP_SREG, sreg);
        attr_be32(&mut d, NFTA_CMP_OP, NFT_CMP_EQ);
        let val = data_value(value);
        attr(&mut d, NFTA_CMP_DATA | NLA_F_NESTED, &val);
        expr("cmp", &d)
    }
    fn e_meta_load(key: u32, dreg: u32) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_META_DREG, dreg);
        attr_be32(&mut d, NFTA_META_KEY, key);
        expr("meta", &d)
    }
    fn e_meta_set(key: u32, sreg: u32) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_META_KEY, key);
        attr_be32(&mut d, NFTA_META_SREG, sreg);
        expr("meta", &d)
    }
    fn e_immediate_value(dreg: u32, value: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_IMMEDIATE_DREG, dreg);
        let val = data_value(value);
        attr(&mut d, NFTA_IMMEDIATE_DATA | NLA_F_NESTED, &val);
        expr("immediate", &d)
    }
    fn e_immediate_verdict(code: u32) -> Vec<u8> {
        let mut verdict = Vec::new();
        attr_be32(&mut verdict, NFTA_VERDICT_CODE, code);
        let vd = nested(NFTA_DATA_VERDICT, &verdict);
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_IMMEDIATE_DREG, NFT_REG_VERDICT);
        attr(&mut d, NFTA_IMMEDIATE_DATA | NLA_F_NESTED, &vd);
        expr("immediate", &d)
    }
    fn e_tproxy(reg_addr: u32, reg_port: u32) -> Vec<u8> {
        let mut d = Vec::new();
        attr_be32(&mut d, NFTA_TPROXY_FAMILY, NFPROTO_IPV4);
        attr_be32(&mut d, NFTA_TPROXY_REG_ADDR, reg_addr);
        attr_be32(&mut d, NFTA_TPROXY_REG_PORT, reg_port);
        expr("tproxy", &d)
    }

    /// The exact expression list for
    ///   ip daddr <vip> tcp dport <vport>
    ///   tproxy to 127.0.0.1:<agent_port> meta mark set <mark> accept
    fn tproxy_rule_expressions(vip: Ipv4Addr, vport: u16, agent_port: u16, mark: u32) -> Vec<u8> {
        let mut ex = Vec::new();
        // ip daddr <vip>: payload(network,16,4)->reg1 ; cmp reg1 == vip (network order)
        ex.extend(e_payload(NFT_PAYLOAD_NETWORK_HEADER, 16, 4, NFT_REG_1));
        ex.extend(e_cmp_eq(NFT_REG_1, &vip.octets()));
        // tcp: meta l4proto -> reg1 ; cmp reg1 == 6 (1 byte)
        ex.extend(e_meta_load(NFT_META_L4PROTO, NFT_REG_1));
        ex.extend(e_cmp_eq(NFT_REG_1, &[IPPROTO_TCP]));
        // dport <vport>: payload(transport,2,2)->reg1 ; cmp reg1 == vport (be16)
        ex.extend(e_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 2, 2, NFT_REG_1));
        ex.extend(e_cmp_eq(NFT_REG_1, &vport.to_be_bytes()));
        // load tproxy dst: reg1 = 127.0.0.1 (network order), reg2 = agent_port (be16)
        ex.extend(e_immediate_value(NFT_REG_1, &Ipv4Addr::new(127, 0, 0, 1).octets()));
        ex.extend(e_immediate_value(NFT_REG_2, &agent_port.to_be_bytes()));
        // tproxy verb: family ipv4, reg_addr=reg1, reg_port=reg2
        ex.extend(e_tproxy(NFT_REG_1, NFT_REG_2));
        // meta mark set <mark>: reg3 = mark (HOST order — matched by the fwmark ip rule) ; meta mark set sreg=reg3
        ex.extend(e_immediate_value(NFT_REG_3, &mark.to_ne_bytes()));
        ex.extend(e_meta_set(NFT_META_MARK, NFT_REG_3));
        // accept
        ex.extend(e_immediate_verdict(NF_ACCEPT));
        ex
    }

    fn nlmsg(buf: &mut Vec<u8>, typ: u16, flags: u16, seq: u32, payload: &[u8]) {
        let len = 16 + payload.len();
        buf.extend_from_slice(&(len as u32).to_ne_bytes());
        buf.extend_from_slice(&typ.to_ne_bytes());
        buf.extend_from_slice(&flags.to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes()); // pid 0 = kernel
        buf.extend_from_slice(payload);
        pad4(buf);
    }
    fn nfgenmsg(family: u8, res_id: u16) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(family);
        v.push(0); // NFNETLINK_V0
        v.extend_from_slice(&res_id.to_be_bytes()); // __be16
        v
    }

    /// Returns the raw NEWRULE payload bytes (nfgenmsg + rule attrs) — also handed
    /// back so the probe can hex-dump the exact tproxy expression it sent.
    pub fn build_newrule_payload(
        table: &str,
        chain: &str,
        vip: Ipv4Addr,
        vport: u16,
        agent_port: u16,
        mark: u32,
    ) -> Vec<u8> {
        let exprs = tproxy_rule_expressions(vip, vport, agent_port, mark);
        let mut payload = nfgenmsg(NFPROTO_IPV4 as u8, 0);
        let mut tz = table.as_bytes().to_vec();
        tz.push(0);
        attr(&mut payload, NFTA_RULE_TABLE, &tz);
        let mut cz = chain.as_bytes().to_vec();
        cz.push(0);
        attr(&mut payload, NFTA_RULE_CHAIN, &cz);
        attr(&mut payload, NFTA_RULE_EXPRESSIONS | NLA_F_NESTED, &exprs);
        payload
    }

    /// Extract JUST the hand-rolled tproxy expression bytes for evidence.
    pub fn tproxy_expr_bytes() -> Vec<u8> {
        e_tproxy(NFT_REG_1, NFT_REG_2)
    }

    /// Send BATCH_BEGIN + NEWRULE + BATCH_END over NETLINK_NETFILTER and check the
    /// ACK. Ok(()) = kernel accepted; Err = the netlink error (e.g. EINVAL on a bad
    /// tproxy encoding, EOPNOTSUPP if nft_tproxy is not built into the kernel).
    pub fn send_newrule(payload: &[u8]) -> Result<(), String> {
        let mut batch = Vec::new();
        nlmsg(&mut batch, NFNL_MSG_BATCH_BEGIN, NLM_F_REQUEST, 1, &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES));
        nlmsg(
            &mut batch,
            (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_NEWRULE,
            NLM_F_REQUEST | NLM_F_CREATE | NLM_F_APPEND | NLM_F_ACK,
            2,
            payload,
        );
        nlmsg(&mut batch, NFNL_MSG_BATCH_END, NLM_F_REQUEST, 3, &nfgenmsg(AF_UNSPEC, NFNL_SUBSYS_NFTABLES));

        // SAFETY: standard libc netlink socket dance; fd closed before return.
        unsafe {
            let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_NETFILTER);
            if fd < 0 {
                return Err(format!("socket: {}", Error::last_os_error()));
            }
            let mut sa: libc::sockaddr_nl = std::mem::zeroed();
            sa.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            if libc::bind(fd, std::ptr::addr_of!(sa).cast(), std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t) != 0 {
                let e = Error::last_os_error();
                libc::close(fd);
                return Err(format!("bind: {e}"));
            }
            let tv = libc::timeval { tv_sec: 2, tv_usec: 0 };
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, std::ptr::addr_of!(tv).cast(), std::mem::size_of::<libc::timeval>() as libc::socklen_t);
            let mut dst: libc::sockaddr_nl = std::mem::zeroed();
            dst.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            let sent = libc::sendto(
                fd,
                batch.as_ptr().cast(),
                batch.len(),
                0,
                std::ptr::addr_of!(dst).cast(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            );
            if sent < 0 {
                let e = Error::last_os_error();
                libc::close(fd);
                return Err(format!("sendto: {e}"));
            }
            let mut buf = [0u8; 16384];
            let n = libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0);
            libc::close(fd);
            if n <= 0 {
                return Err(format!("no netlink ACK within timeout: {}", Error::last_os_error()));
            }
            // Walk the nlmsghdrs; NLMSG_ERROR(2) carries the batch verdict.
            let n = n as usize;
            let mut off = 0usize;
            while off + 16 <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                let typ = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                if len < 16 || off + len > n {
                    break;
                }
                if typ == 2 {
                    // struct nlmsgerr { int error; struct nlmsghdr msg; }
                    let code = i32::from_ne_bytes(buf[off + 16..off + 20].try_into().unwrap());
                    if code == 0 {
                        return Ok(());
                    }
                    return Err(format!(
                        "netlink NLMSG_ERROR code {code} ({})",
                        Error::from_raw_os_error(-code)
                    ));
                }
                off += (len + 3) & !3;
            }
            // No NLMSG_ERROR seen — treat as accepted (downstream list/divert confirm).
            Ok(())
        }
    }

    /// Hex dump helper for the evidence file.
    pub fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
    }
}

async fn link_index(handle: &Handle, name: &str) -> Option<u32> {
    let mut s = handle.link().get().match_name(name.to_string()).execute();
    match s.try_next().await {
        Ok(Some(msg)) => Some(msg.header.index),
        _ => None,
    }
}

fn write_sysctl(path: &str, val: &str) {
    match std::fs::write(path, val) {
        Ok(()) => info!("sysctl {path} = {val}"),
        Err(e) => info!("sysctl {path} write failed (non-fatal): {e}"),
    }
}

/// IP_TRANSPARENT + SO_REUSEADDR TcpListener bound to 127.0.0.1:port (leg-C stand-in).
fn transparent_listener(port: u16) -> std::io::Result<TcpListener> {
    // SAFETY: standard libc socket construction; fd wrapped into TcpListener on success.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let one: libc::c_int = 1;
        let optlen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, std::ptr::addr_of!(one).cast(), optlen);
        let rc = libc::setsockopt(fd, libc::SOL_IP, libc::IP_TRANSPARENT, std::ptr::addr_of!(one).cast(), optlen);
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = port.to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(Ipv4Addr::new(127, 0, 0, 1).octets());
        if libc::bind(fd, std::ptr::addr_of!(sa).cast(), std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        if libc::listen(fd, 16) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(TcpListener::from_raw_fd(fd))
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    println!("=== PROBE increment-e: nft TPROXY (HAND-ROLLED tproxy rule via raw netlink) + real divert proof ===");
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    println!("kernel: {}", kernel.trim());
    println!("rustables tproxy expression: NONE (no typed tproxy + `nlmsg` is pub(crate) -> no public raw-expr injection); rule HAND-ROLLED over NETLINK_NETFILTER");
    println!(
        "hand-rolled tproxy expr bytes (NFTA_EXPR name=tproxy + FAMILY/REG_ADDR/REG_PORT): {}",
        nft_raw::hex(&nft_raw::tproxy_expr_bytes())
    );

    let mut fails = 0usize;

    // ---------- best-effort pre-clean ----------
    let _ = NetworkNamespace::del(NETNS.to_string()).await;
    {
        let mut b = Batch::new();
        let t = Table::new(ProtocolFamily::Ipv4).with_name(NFT_TABLE);
        b.add(&t, MsgType::Del);
        let _ = b.send();
    }
    let (connection, handle, _) = new_connection().expect("netlink connection");
    tokio::spawn(connection);
    if let Some(idx) = link_index(&handle, HOST_VETH).await {
        let _ = handle.link().del(idx).execute().await;
    }
    preclean_ip_rule_route(&handle).await;

    // ---------- netns + veth + addr + route (rtnetlink) ----------
    step!("rtnetlink: netns {NETNS} + veth {WL_VETH}<->{HOST_VETH}; host {HOST_ADDR}/{PREFIX}; netns {WL_ADDR}/{PREFIX} + default route");
    NetworkNamespace::add(NETNS.to_string()).await.expect("netns add");
    handle.link().add(LinkVeth::new(WL_VETH, HOST_VETH).build()).execute().await.expect("veth add");
    let host_idx = link_index(&handle, HOST_VETH).await.expect("host veth idx");
    let wl_idx = link_index(&handle, WL_VETH).await.expect("wl veth idx");
    handle.link().set(LinkUnspec::new_with_index(host_idx).up().build()).execute().await.expect("host up");
    handle.address().add(host_idx, IpAddr::V4(HOST_ADDR), PREFIX).execute().await.expect("host addr");
    let ns_file = File::open(NETNS_PATH).expect("open netns");
    handle
        .link()
        .set(LinkUnspec::new_with_index(wl_idx).setns_by_fd(ns_file.as_raw_fd()).build())
        .execute()
        .await
        .expect("move wl into netns");
    drop(ns_file);
    let ok_netns = std::thread::spawn(|| -> bool {
        let nsf = File::open(NETNS_PATH).expect("open netns (thread)");
        if unsafe { libc::setns(nsf.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
            return false;
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (c, h, _) = new_connection().unwrap();
            tokio::spawn(c);
            let wl = match link_index(&h, WL_VETH).await {
                Some(i) => i,
                None => return false,
            };
            if h.address().add(wl, IpAddr::V4(WL_ADDR), PREFIX).execute().await.is_err() {
                return false;
            }
            let _ = h.link().set(LinkUnspec::new_with_index(wl).up().build()).execute().await;
            if let Some(lo) = link_index(&h, "lo").await {
                let _ = h.link().set(LinkUnspec::new_with_index(lo).up().build()).execute().await;
            }
            let dflt = RouteMessageBuilder::<Ipv4Addr>::new().gateway(HOST_ADDR).build();
            h.route().add(dflt).execute().await.is_ok()
        })
    })
    .join()
    .unwrap();
    if ok_netns {
        ok!("netns/veth/addr/route up (client reaches {VIP} via default -> {HOST_ADDR})");
    } else {
        bad!("netns setup failed");
        fails += 1;
    }

    // ---------- sysctls ----------
    step!("sysctls: rp_filter=0 (asymmetric TPROXY path)");
    write_sysctl("/proc/sys/net/ipv4/conf/all/rp_filter", "0");
    write_sysctl("/proc/sys/net/ipv4/conf/default/rp_filter", "0");
    write_sysctl(&format!("/proc/sys/net/ipv4/conf/{HOST_VETH}/rp_filter"), "0");

    // ---------- ip rule fwmark + ip route local (rtnetlink; increment-d) ----------
    step!("rtnetlink: ip rule add fwmark {TPROXY_FWMARK:#x} lookup {TPROXY_RT_TABLE} + ip route add local 0.0.0.0/0 dev lo table {TPROXY_RT_TABLE}");
    let lo_idx = link_index(&handle, "lo").await.expect("lo idx");
    if count_fwmark_rules(&handle).await == 0 {
        match handle.rule().add().v4().action(RuleAction::ToTable).fw_mark(TPROXY_FWMARK).table_id(TPROXY_RT_TABLE).execute().await {
            Ok(()) => info!("fwmark rule added"),
            Err(e) => {
                bad!("fwmark rule add failed: {e}");
                fails += 1;
            }
        }
    }
    match handle.route().add(build_local_route(lo_idx)).execute().await {
        Ok(()) => info!("local route added"),
        Err(rtnetlink::Error::NetlinkError(e)) if e.raw_code().abs() == libc::EEXIST => info!("local route already present"),
        Err(e) => {
            bad!("local route add failed: {e}");
            fails += 1;
        }
    }

    // ---------- nft table + prerouting chain + exemption (rustables) ----------
    step!("rustables: table + prerouting(type filter hook prerouting priority mangle) + `meta mark {MTLS_LEG_S_DIAL_MARK:#x} accept` exemption");
    let table = Table::new(ProtocolFamily::Ipv4).with_name(NFT_TABLE);
    let chain = Chain::new(&table)
        .with_name(NFT_CHAIN)
        .with_hook(Hook::new(HookClass::PreRouting, -150)) // priority mangle = -150
        .with_type(ChainType::Filter)
        .with_policy(ChainPolicy::Accept);
    let mut batch = Batch::new();
    table.add_to_batch(&mut batch);
    let chain = chain.add_to_batch(&mut batch);
    let mut exemption = Rule::new(&chain).expect("exemption rule");
    exemption.add_expr(Meta::new(MetaType::Mark)); // load skb mark -> reg1
    exemption.add_expr(Cmp::new(CmpOp::Eq, MTLS_LEG_S_DIAL_MARK.to_ne_bytes().to_vec())); // == 0x2 (host order)
    let exemption = exemption.accept();
    exemption.add_to_batch(&mut batch);
    match batch.send() {
        Ok(()) => ok!("rustables installed table + prerouting chain + leg-S exemption"),
        Err(e) => {
            bad!("rustables table/chain/exemption send failed: {e:?}");
            fails += 1;
        }
    }

    // ---------- HAND-ROLLED tproxy rule (raw NETLINK_NETFILTER) ----------
    step!("HAND-ROLLED raw netlink: NEWRULE `ip daddr {VIP} tcp dport {VPORT} tproxy to 127.0.0.1:{AGENT_PORT} meta mark set {TPROXY_FWMARK:#x} accept`");
    let payload = nft_raw::build_newrule_payload(NFT_TABLE, NFT_CHAIN, VIP, VPORT, AGENT_PORT, TPROXY_FWMARK);
    info!("NEWRULE payload {} bytes", payload.len());
    match nft_raw::send_newrule(&payload) {
        Ok(()) => ok!("kernel ACCEPTED the hand-rolled tproxy rule (valid nft_tproxy encoding on this kernel)"),
        Err(e) => {
            bad!("kernel REJECTED the hand-rolled tproxy rule: {e}");
            fails += 1;
        }
    }

    // ---------- structural handle recovery (rustables list_rules_for_chain) ----------
    step!("structural handle recovery: list_rules_for_chain (vs production `# handle N` text parse)");
    let mut tproxy_handle: Option<u64> = None;
    match list_rules_for_chain(&chain) {
        Ok(rules) => {
            let handles: Vec<u64> = rules.iter().filter_map(|r| r.get_handle().copied()).collect();
            info!("chain carries {} rule(s); kernel handles {handles:?}", rules.len());
            tproxy_handle = handles.last().copied(); // tproxy rule appended after the exemption
            match tproxy_handle {
                Some(h) => ok!("recovered tproxy rule handle {h} structurally from the NEWRULE dump"),
                None => {
                    bad!("no rule handle recovered");
                    fails += 1;
                }
            }
        }
        Err(e) => {
            bad!("list_rules_for_chain failed: {e:?}");
            fails += 1;
        }
    }

    // ---------- EVIDENCE-ONLY oracle: nft -a list chain ----------
    step!("EVIDENCE-ONLY oracle: `nft -a list chain ip {NFT_TABLE} {NFT_CHAIN}` (nft(8) read-only, NOT the mechanism)");
    match std::process::Command::new("nft").args(["-a", "list", "chain", "ip", NFT_TABLE, NFT_CHAIN]).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                println!("    nft| {line}");
            }
            if text.contains(&format!("tproxy to 127.0.0.1:{AGENT_PORT}")) {
                ok!("nft rendered our hand-rolled bytes as a real `tproxy to 127.0.0.1:{AGENT_PORT}` rule");
            } else {
                bad!("nft did NOT render the expected tproxy verb — hand-rolled encoding suspect");
                fails += 1;
            }
        }
        Err(e) => info!("nft evidence oracle unavailable ({e}); relying on structural readback + divert proof"),
    }

    // ---------- THE PROOF ----------
    step!("PROOF: netns client connects to {VIP}:{VPORT}; must be TPROXY-diverted to the 127.0.0.1:{AGENT_PORT} transparent listener");
    let divert_ok = run_divert_proof(&mut fails);

    // ---------- delete-by-handle (rustables) ----------
    if let Some(h) = tproxy_handle {
        step!("nft delete rule ip {NFT_TABLE} {NFT_CHAIN} handle {h}  (by-handle teardown, rustables)");
        let mut b = Batch::new();
        let del = Rule::new(&chain).expect("del rule").with_handle(h);
        b.add(&del, MsgType::Del);
        match b.send() {
            Ok(()) => {
                let remaining = list_rules_for_chain(&chain).map(|v| v.len()).unwrap_or(usize::MAX);
                if remaining == 1 {
                    ok!("by-handle delete removed exactly the tproxy rule (exemption remains; 2 -> 1)");
                } else {
                    bad!("after by-handle delete, chain has {remaining} rule(s) (expected 1)");
                    fails += 1;
                }
            }
            Err(e) => {
                bad!("by-handle delete failed: {e:?}");
                fails += 1;
            }
        }
    }

    // ---------- cleanup ----------
    step!("cleanup: nft table del + ip rule/route del + netns del + veth del + rp_filter restore");
    {
        let mut b = Batch::new();
        let t = Table::new(ProtocolFamily::Ipv4).with_name(NFT_TABLE);
        b.add(&t, MsgType::Del);
        let _ = b.send();
    }
    preclean_ip_rule_route(&handle).await;
    let _ = NetworkNamespace::del(NETNS.to_string()).await;
    if let Some(idx) = link_index(&handle, HOST_VETH).await {
        let _ = handle.link().del(idx).execute().await;
    }
    write_sysctl("/proc/sys/net/ipv4/conf/all/rp_filter", "1");
    write_sysctl("/proc/sys/net/ipv4/conf/default/rp_filter", "1");
    ok!("cleanup done");

    println!("\n=== VERDICT (increment-e) ===");
    if fails == 0 && divert_ok {
        println!(
            "WORKS — rustables 0.8.8 has NO tproxy expression and NO public raw-expr escape hatch, so the tproxy rule was HAND-ROLLED over raw NETLINK_NETFILTER; the kernel ACCEPTED it, nft rendered it as a real `tproxy to 127.0.0.1:{AGENT_PORT}`, a real netns connection to {VIP}:{VPORT} was DIVERTED to the transparent listener with orig-dst preserved and a byte-distinct round-trip, and the rule was recovered + deleted by kernel handle. Zero `nft`/`ip` subprocesses on the mechanism path."
        );
    } else {
        println!("DOESN'T-WORK — {fails} check(s) failed; divert_ok={divert_ok}. See [FAIL] lines + findings.");
    }
    std::process::exit(i32::from(fails != 0 || !divert_ok));
}

fn run_divert_proof(fails: &mut usize) -> bool {
    let listener = match transparent_listener(AGENT_PORT) {
        Ok(l) => l,
        Err(e) => {
            bad!("transparent listener bind failed: {e}");
            *fails += 1;
            return false;
        }
    };

    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (res_tx, res_rx) = mpsc::channel::<Result<SocketAddr, String>>();

    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        ready_tx.send(()).ok();
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        loop {
            match listener.accept() {
                Ok((mut sock, _peer)) => {
                    let local = sock.local_addr();
                    sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let mut buf = [0u8; 64];
                    let n = sock.read(&mut buf).unwrap_or(0);
                    if &buf[..n] != REQUEST {
                        res_tx.send(Err(format!("request mismatch: got {:?}", &buf[..n]))).ok();
                        return;
                    }
                    sock.write_all(RESPONSE).ok();
                    sock.flush().ok();
                    match local {
                        Ok(a) => res_tx.send(Ok(a)).ok(),
                        Err(e) => res_tx.send(Err(format!("local_addr: {e}"))).ok(),
                    };
                    return;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() > deadline {
                        res_tx.send(Err("accept timed out (no divert)".into())).ok();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    res_tx.send(Err(format!("accept: {e}"))).ok();
                    return;
                }
            }
        }
    });

    ready_rx.recv_timeout(Duration::from_secs(2)).ok();
    std::thread::sleep(Duration::from_millis(150));

    let client = std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let nsf = File::open(NETNS_PATH).map_err(|e| format!("open netns: {e}"))?;
        if unsafe { libc::setns(nsf.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
            return Err(format!("setns: {}", std::io::Error::last_os_error()));
        }
        let target = SocketAddr::V4(SocketAddrV4::new(VIP, VPORT));
        let mut stream = TcpStream::connect_timeout(&target, Duration::from_secs(4)).map_err(|e| format!("connect {target}: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();
        stream.write_all(REQUEST).map_err(|e| format!("write: {e}"))?;
        stream.flush().ok();
        let mut buf = vec![0u8; 64];
        let n = stream.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        buf.truncate(n);
        Ok(buf)
    });

    let server_res = res_rx.recv_timeout(Duration::from_secs(8));
    let client_res = client.join().unwrap_or_else(|_| Err("client panicked".into()));
    let _ = server.join();

    let mut good = true;
    match &server_res {
        Ok(Ok(local)) => {
            let want = SocketAddr::V4(SocketAddrV4::new(VIP, VPORT));
            if *local == want {
                ok!("DIVERTED: accept fired; accepted socket local_addr == {want} (orig-dst preserved — a FOREIGN-dst connection was TPROXY-diverted to 127.0.0.1:{AGENT_PORT})");
            } else {
                bad!("accept fired but local_addr {local} != {want} (orig-dst NOT preserved)");
                good = false;
            }
        }
        Ok(Err(e)) => {
            bad!("server side: {e}");
            good = false;
        }
        Err(_) => {
            bad!("server produced no result within timeout (connection was NOT diverted)");
            good = false;
        }
    }
    match &client_res {
        Ok(resp) if resp.as_slice() == RESPONSE => ok!("byte-distinct round-trip: client received RESPONSE verbatim ({} bytes)", resp.len()),
        Ok(resp) => {
            bad!("client got unexpected bytes: {resp:?}");
            good = false;
        }
        Err(e) => {
            bad!("client side: {e}");
            good = false;
        }
    }
    if !good {
        *fails += 1;
    }
    good
}

async fn count_fwmark_rules(handle: &Handle) -> usize {
    let mut s = handle.rule().get(IpVersion::V4).execute();
    let mut n = 0;
    while let Ok(Some(msg)) = s.try_next().await {
        let has_mark = msg.attributes.iter().any(|a| matches!(a, RuleAttribute::FwMark(m) if *m == TPROXY_FWMARK));
        let has_table = u32::from(msg.header.table) == TPROXY_RT_TABLE
            || msg.attributes.iter().any(|a| matches!(a, RuleAttribute::Table(t) if *t == TPROXY_RT_TABLE));
        if has_mark && has_table {
            n += 1;
        }
    }
    n
}

async fn preclean_ip_rule_route(handle: &Handle) {
    let mut s = handle.rule().get(IpVersion::V4).execute();
    let mut victims = Vec::new();
    while let Ok(Some(msg)) = s.try_next().await {
        let has_mark = msg.attributes.iter().any(|a| matches!(a, RuleAttribute::FwMark(m) if *m == TPROXY_FWMARK));
        let has_table = u32::from(msg.header.table) == TPROXY_RT_TABLE
            || msg.attributes.iter().any(|a| matches!(a, RuleAttribute::Table(t) if *t == TPROXY_RT_TABLE));
        if has_mark && has_table {
            victims.push(msg);
        }
    }
    for v in victims {
        let _ = handle.rule().del(v).execute().await;
    }
    if let Some(lo) = link_index(handle, "lo").await {
        let _ = handle.route().del(build_local_route(lo)).execute().await;
    }
}

fn build_local_route(lo_idx: u32) -> rtnetlink::packet_route::route::RouteMessage {
    RouteMessageBuilder::<Ipv4Addr>::new()
        .kind(RouteType::Local)
        .scope(RouteScope::Host)
        .table_id(TPROXY_RT_TABLE)
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .output_interface(lo_idx)
        .build()
}
