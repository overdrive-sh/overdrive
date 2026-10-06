//! Flow owner for both sides. Control plane only: creates/accepts/connects sockets,
//! exchanges fixed-size control messages (16 B request, 8 B control-channel
//! messages, optional 4 B in-band ack for the ADR-0158 comparison), installs
//! SockHash entries and routes, and propagates EOF/RST by readiness. It never
//! reads or writes application payload on a paired socket.
use crate::sys::*;
use aya::{
    maps::{Array, HashMap, MapData, PerCpuArray, PerCpuHashMap, Queue, SockHash},
    programs::{CgroupAttachMode, CgroupSock, CgroupSockAddr, FExit, SchedClassifier, SkSkb, SockOps, TcAttachType},
    Btf, Ebpf, EbpfLoader,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap as StdMap};
use std::fs::File;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

pub const CTL_PORT: u32 = 1234;
pub const FLOW_PORT: u32 = 1235;
pub const SEQ_PORT: u32 = 1236;
pub const GUEST_ACCEPT_PORT: u32 = 5000;
pub const MARK: i32 = 0x295;

const K_TCP_CONNECT: u8 = 1;
const K_TCP_ACCEPT: u8 = 2;
const K_DG_STREAM: u8 = 3;
const K_DG_SEQ: u8 = 4;
const M_PAIRED: u8 = 1;
const M_REFUSED: u8 = 2;
const M_ABORT: u8 = 3;
const M_HELLO: u8 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct Route {
    target: u64,
    mode: u32,
    pad: u32,
}
unsafe impl aya::Pod for Route {}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Dst {
    ip: u32,
    port: u32,
}
unsafe impl aya::Pod for Dst {}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Slot {
    cookie: u64,
    ip: u32,
    port: u32,
    app_port: u32,
    gen: u32,
}
unsafe impl aya::Pod for Slot {}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Tuple {
    src: u32,
    dst: u32,
    sport: u16,
    dport: u16,
}
unsafe impl aya::Pod for Tuple {}

pub struct Cfg {
    pub role: String,
    pub ack_oob: bool,
    pub flush: bool,
    pub udp_park: bool,
    pub intake_park: bool,
    pub arm: bool,
    pub park_pool: u32,
    pub w: Ipv4Addr,
    pub tcp_intake: u16,
    pub udp_base: u16,
    pub udp_slots: u32,
    pub dns: SocketAddrV4,
    pub cid: u32,
    pub intakes: Vec<(SocketAddrV4, u16)>,
    pub log: String,
    pub stats: String,
    pub obj: String,
    pub verbose_flows: u32,
}

pub fn parse_cfg(args: &[String]) -> Cfg {
    let mut c = Cfg {
        role: args[0].clone(),
        ack_oob: true,
        flush: true,
        udp_park: true,
        intake_park: true,
        arm: true,
        park_pool: 256,
        w: Ipv4Addr::new(10, 99, 7, 2),
        tcp_intake: 15001,
        udp_base: 20000,
        udp_slots: 64,
        dns: "127.0.0.53:53".parse().unwrap(),
        cid: 0,
        intakes: vec![],
        log: "/tmp/owner.log".into(),
        stats: "/tmp/owner-stats.json".into(),
        obj: "/vfwd-bpf.o".into(),
        verbose_flows: 50,
    };
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--ack" => c.ack_oob = v == "oob",
            "--flush" => c.flush = v == "yes",
            "--udp" => c.udp_park = v == "park",
            "--intake-mode" => c.intake_park = v == "park",
            "--arm" => c.arm = v == "yes",
            "--park-pool" => c.park_pool = v.parse().unwrap(),
            "--w" => c.w = v.parse().unwrap(),
            "--udp-slots" => c.udp_slots = v.parse().unwrap(),
            "--cid" => c.cid = v.parse().unwrap(),
            "--intake" => {
                let (a, g) = v.split_once('=').unwrap();
                c.intakes.push((a.parse().unwrap(), g.parse().unwrap()));
            }
            "--log" => c.log = v,
            "--stats" => c.stats = v,
            "--obj" => c.obj = v,
            "--verbose-flows" => c.verbose_flows = v.parse().unwrap(),
            x => panic!("unknown arg {x}"),
        }
        i += 2;
    }
    c
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LegKind {
    Tcp,
    Vsock,
}

struct Leg {
    fd: OwnedFd,
    cookie: u64,
    kind: LegKind,
    eof: bool,
    shut: bool,
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum FState {
    AwaitAck,    // opener: waiting for Paired
    Active,
}

struct Flow {
    kind: u8,
    state: FState,
    legs: [Option<Leg>; 2], // [0] = inet side, [1] = vsock side
    drain: [Option<Instant>; 2], // direction src leg index -> started
    park: Option<usize>, // kernel TCP parking cell for the intake leg
    created: Instant,
}

struct Park {
    p1: OwnedFd,
    p2: OwnedFd,
    p1ck: u64,
    p2ck: u64,
    /// cumulative counters when the cell was last released: (SENT p1, bytes_received p2, FWD p2)
    base: (u64, u64, u64),
}

struct DgPending {
    stream: Option<OwnedFd>,
    seq: Option<OwnedFd>,
    dst: SocketAddrV4,
    since: Instant,
}

struct HostDg {
    h: OwnedFd,
    t: OwnedFd,
    s: OwnedFd,
    tuple: Tuple,
}

struct GuestSlot {
    up: OwnedFd,
    p1: OwnedFd,
    p2: OwnedFd,
    p3: OwnedFd, // reply cell: vsock STREAM -> p3 (egress) -> p4 (strparser: one frame per verdict) -> up
    p4: OwnedFd,
    assoc: Option<(u32, OwnedFd, OwnedFd)>, // flow id, stream, seqpacket
    active: bool,
    tuple: Option<Tuple>,
    released: bool,
    installed_naive: bool,
    retry_at: Option<Instant>,
}

pub struct Owner {
    c: Cfg,
    bpf: Ebpf,
    ep: Epoll,
    log: File,
    ctl: StdMap<u32, OwnedFd>, // cid -> control socket (host); guest uses key 2
    ctl_in: StdMap<RawFd, (u32, Vec<u8>)>,
    flows: BTreeMap<u32, Flow>,
    fdmap: StdMap<RawFd, (u32, usize)>,
    listeners: StdMap<RawFd, (OwnedFd, &'static str, u16)>,
    next_flow: u32,
    dg_pending: StdMap<u32, DgPending>,
    host_dg: StdMap<u32, HostDg>,
    hostdg_fd: StdMap<RawFd, u32>,
    slots: Vec<GuestSlot>,
    p2fd: StdMap<RawFd, usize>,
    assocfd: StdMap<RawFd, usize>,
    flow_slot: StdMap<u32, usize>,
    stat: BTreeMap<&'static str, u64>,
    last_stats: Instant,
    pending_inband: StdMap<RawFd, u32>,
    parks: Vec<Park>,
}

fn sockhash<'a>(bpf: &'a mut Ebpf, name: &str) -> SockHash<&'a mut MapData, u64> {
    SockHash::try_from(bpf.map_mut(name).unwrap()).unwrap()
}

impl Owner {
    fn ev(&mut self, v: serde_json::Value) {
        let _ = writeln!(self.log, "{}", v);
    }
    fn bump(&mut self, k: &'static str, n: u64) {
        *self.stat.entry(k).or_insert(0) += n;
    }

    // ---------------------------------------------------------------- map plumbing
    fn add_sock(&mut self, map: &str, fd: RawFd) -> u64 {
        let ck = cookie(fd);
        let mut m = sockhash(&mut self.bpf, map);
        if let Err(e) = m.insert(ck, fd, 0) {
            let kind = local4(fd).map(|a| a.to_string()).unwrap_or_else(|_| "non-inet".into());
            let msg = format!("{e:?}");
            self.bump("sockhash_insert_errors", 1);
            self.ev(json!({"ev":"sockhash_insert_failed","map":map,"fd":fd,"sock":kind,"err":msg}));
            eprintln!("sockhash_insert_failed map={map} fd={fd} sock={kind} err={msg}");
        }
        ck
    }
    fn del_sock(&mut self, map: &str, ck: u64) {
        let mut m = sockhash(&mut self.bpf, map);
        let _ = m.remove(&ck);
    }
    fn route(&mut self, src: u64, target: u64, mode: u32) {
        let mut r: HashMap<_, u64, Route> = HashMap::try_from(self.bpf.map_mut("ROUTE").unwrap()).unwrap();
        r.insert(src, Route { target, mode, pad: 0 }, 0).unwrap();
    }
    fn unroute(&mut self, src: u64) {
        let mut r: HashMap<_, u64, Route> = HashMap::try_from(self.bpf.map_mut("ROUTE").unwrap()).unwrap();
        let _ = r.remove(&src);
    }
    /// Forget K2 counters of a socket that is being closed (never for long-lived cells).
    fn forget_counters(&mut self, ck: u64) {
        for name in ["FWD", "SENT"] {
            let mut m: PerCpuHashMap<_, u64, u64> = PerCpuHashMap::try_from(self.bpf.map_mut(name).unwrap()).unwrap();
            let _ = m.remove(&ck);
        }
    }
    fn percpu(&self, map: &str, k: u64) -> u64 {
        let m: PerCpuHashMap<_, u64, u64> = PerCpuHashMap::try_from(self.bpf.map(map).unwrap()).unwrap();
        m.get(&k, 0).map(|v| v.iter().sum()).unwrap_or(0)
    }
    fn counters(&self) -> Vec<u64> {
        let a: PerCpuArray<_, u64> = PerCpuArray::try_from(self.bpf.map("CNT").unwrap()).unwrap();
        (0..32u32).map(|i| a.get(&i, 0).unwrap().iter().sum()).collect()
    }
    fn tuple_set(&mut self, t: Tuple, v: Option<u32>) {
        let mut m: HashMap<_, Tuple, u32> = HashMap::try_from(self.bpf.map_mut("TUPLES").unwrap()).unwrap();
        match v {
            Some(v) => m.insert(t, v, 0).unwrap(),
            None => {
                let _ = m.remove(&t);
            }
        }
    }

    /// Activate stream_verdict on an already-installed TCP socket and re-arm data_ready
    /// (SO_RCVLOWAT -> tcp_set_rcvlowat -> tcp_data_ready) so bytes queued before
    /// install are forwarded (K1 for TCP).
    fn arm(&mut self, fd: RawFd, tcp: bool) {
        self.add_sock("VERD", fd);
        if tcp && self.c.flush {
            setopt_i32(fd, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
            self.bump("rcvlowat_rearm", 1);
        }
    }

    // ---------------------------------------------------------------- control channel
    fn send_ctl(&mut self, cid: u32, ty: u8, flow: u32) {
        let mut m = [0u8; 8];
        m[0] = ty;
        m[4..8].copy_from_slice(&flow.to_le_bytes());
        if let Some(fd) = self.ctl.get(&cid) {
            let _ = write_all_ctl(fd.as_raw_fd(), &m);
            self.bump("ctl_bytes_written", 8);
        }
    }

    fn request(kind: u8, ip: Ipv4Addr, port: u16, flow: u32, port2: u16) -> [u8; 16] {
        let mut r = [0u8; 16];
        r[0] = kind;
        r[1] = 1;
        r[2..4].copy_from_slice(&port.to_be_bytes());
        r[4..8].copy_from_slice(&ip.octets());
        r[8..12].copy_from_slice(&flow.to_le_bytes());
        r[12..14].copy_from_slice(&port2.to_be_bytes());
        r
    }

    // ---------------------------------------------------------------- flows
    fn new_flow(&mut self, id: u32, kind: u8, state: FState) {
        self.flows.insert(id, Flow { kind, state, legs: [None, None], drain: [None, None], park: None, created: Instant::now() });
    }
    fn set_leg(&mut self, id: u32, i: usize, fd: OwnedFd, kind: LegKind) {
        let raw = fd.as_raw_fd();
        let ck = cookie(raw);
        self.fdmap.insert(raw, (id, i));
        self.flows.get_mut(&id).unwrap().legs[i] = Some(Leg { fd, cookie: ck, kind, eof: false, shut: false });
    }
    fn leg_fd(&self, id: u32, i: usize) -> RawFd {
        self.flows[&id].legs[i].as_ref().unwrap().fd.as_raw_fd()
    }
    fn leg_ck(&self, id: u32, i: usize) -> u64 {
        self.flows[&id].legs[i].as_ref().unwrap().cookie
    }
    fn watch(&mut self, id: u32) {
        for i in 0..2 {
            let fd = self.leg_fd(id, i);
            self.ep.add(fd, (libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLET) as u32);
        }
    }
    /// Opener side, after Paired: start forwarding the inet leg's bytes into the vsock leg.
    /// Parked intake: route the parking cell's far end (P2) to the vsock leg and re-arm
    /// it (SO_RCVLOWAT) so every byte parked so far is forwarded. Late intake: install now.
    fn activate_inet(&mut self, id: u32) {
        let v_ck = self.leg_ck(id, 1);
        if let Some(slot) = self.flows[&id].park {
            let (p2, p2ck) = (self.parks[slot].p2.as_raw_fd(), self.parks[slot].p2ck);
            self.route(p2ck, v_ck, 0);
            self.add_sock("VERD", p2);
            setopt_i32(p2, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
            self.bump("park_flush", 1);
        } else {
            let (i_fd, i_ck) = (self.leg_fd(id, 0), self.leg_ck(id, 0));
            self.route(i_ck, v_ck, 0);
            self.arm(i_fd, true);
        }
        self.flows.get_mut(&id).unwrap().state = FState::Active;
        self.watch(id);
        self.bump("flows_active_total", 1);
    }

    /// Install the vsock leg (route -> inet leg). The inet leg must already be a SOCKS member.
    fn install_vsock(&mut self, id: u32) {
        let (v_fd, v_ck, i_ck) = (self.leg_fd(id, 1), self.leg_ck(id, 1), self.leg_ck(id, 0));
        self.add_sock("SOCKS", v_fd);
        self.route(v_ck, i_ck, 0);
        self.add_sock("VERD", v_fd);
    }

    /// Late install of an already-connected TCP leg (ADR-0158 shape). Fails if the
    /// socket already left ESTABLISHED (e.g. peer FIN arrived first).
    fn late_install_inet(&mut self, id: u32, flush: bool) -> bool {
        let (i_fd, i_ck, v_ck) = (self.leg_fd(id, 0), self.leg_ck(id, 0), self.leg_ck(id, 1));
        let before = self.stat.get("sockhash_insert_errors").copied().unwrap_or(0);
        self.add_sock("SOCKS", i_fd);
        if self.stat.get("sockhash_insert_errors").copied().unwrap_or(0) != before {
            self.bump("late_install_impossible", 1);
            return false;
        }
        self.route(i_ck, v_ck, 0);
        self.add_sock("VERD", i_fd);
        if flush && self.c.flush {
            setopt_i32(i_fd, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
            self.bump("rcvlowat_rearm", 1);
        }
        true
    }

    fn arm_active(&mut self, fd: RawFd, target: u64) {
        let ck = cookie(fd);
        self.route(ck, target, 0);
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(self.bpf.map_mut("ARM").unwrap()).unwrap();
        m.insert(ck, 1, 0).unwrap();
    }

    fn release_park(&mut self, slot: usize) {
        let (p2, p2ck, p1ck) = (self.parks[slot].p2.as_raw_fd(), self.parks[slot].p2ck, self.parks[slot].p1ck);
        self.unroute(p2ck);
        self.del_sock("VERD", p2ck);
        let old = self.parks[slot].base;
        let new = (self.percpu("SENT", p1ck), tcp_bytes_received(p2).unwrap_or(0), self.percpu("FWD", p2ck));
        self.parks[slot].base = new;
        // clean iff everything P2 received for this flow was forwarded by its verdict
        if new.1 - old.1 == new.2 - old.2 {
            let mut q: Queue<_, u32> = Queue::try_from(self.bpf.map_mut("TCPFREE").unwrap()).unwrap();
            let _ = q.push(slot as u32, 0);
        } else {
            self.bump("park_slot_leaked_nonempty", 1);
        }
    }

    fn close_flow(&mut self, id: u32, reset: bool, why: &str) {
        let Some(mut f) = self.flows.remove(&id) else { return };
        if let Some(slot) = f.park {
            self.release_park(slot);
        }
        for i in 0..2 {
            if let Some(l) = f.legs[i].take() {
                let raw = l.fd.as_raw_fd();
                self.ep.del(raw);
                self.fdmap.remove(&raw);
                self.pending_inband.remove(&raw);
                self.unroute(l.cookie);
                self.forget_counters(l.cookie);
                {
                    let mut m: HashMap<_, u64, u32> = HashMap::try_from(self.bpf.map_mut("ARM").unwrap()).unwrap();
                    let _ = m.remove(&l.cookie);
                    let mut m: HashMap<_, u64, u32> = HashMap::try_from(self.bpf.map_mut("PARK_OF").unwrap()).unwrap();
                    let _ = m.remove(&l.cookie);
                }
                if reset && l.kind == LegKind::Tcp {
                    linger0_close(l.fd);
                } else {
                    drop(l.fd);
                }
            }
        }
        self.bump(if reset { "flows_aborted" } else { "flows_closed" }, 1);
        if id < self.c.verbose_flows || reset {
            let age = f.created.elapsed().as_millis() as u64;
            self.ev(json!({"ev":"flow_closed","flow":id,"kind":f.kind,"reset":reset,"why":why,"age_ms":age}));
        }
    }

    fn abort_flow(&mut self, id: u32, why: &str) {
        let cid = self.peer_cid();
        self.send_ctl(cid, M_ABORT, id);
        self.close_flow(id, true, why);
    }

    fn peer_cid(&self) -> u32 {
        if self.c.role == "guest" {
            2
        } else {
            self.c.cid
        }
    }

    // ---------------------------------------------------------------- EOF / RST (A-10, K2)
    fn on_leg_event(&mut self, fd: RawFd, ev: u32) {
        let Some(&(id, i)) = self.fdmap.get(&fd) else { return };
        if !self.flows.contains_key(&id) {
            return;
        }
        let err = so_error(fd);
        let hup = ev & libc::EPOLLHUP as u32 != 0;
        let rdhup = ev & libc::EPOLLRDHUP as u32 != 0;
        let is_err = ev & libc::EPOLLERR as u32 != 0;
        if is_err || err != 0 {
            self.bump("leg_errors", 1);
            self.ev(json!({"ev":"leg_error","flow":id,"leg":i,"so_error":err,"events":ev}));
            self.abort_flow(id, "leg_error");
            return;
        }
        if rdhup || hup {
            let f = self.flows.get_mut(&id).unwrap();
            let leg = f.legs[i].as_mut().unwrap();
            if !leg.eof {
                leg.eof = true;
                f.drain[i] = Some(Instant::now());
            }
        }
    }

    /// K2: a direction is drained when every hop from the EOF'd leg to its partner has
    /// forwarded (SK_SKB verdict count) everything its socket received, and the next
    /// socket's skb_send_sock count (fexit) has caught up. Only then shutdown(SHUT_WR).
    fn hop_drained(&self, id: u32, i: usize) -> (bool, serde_json::Value) {
        let j = 1 - i;
        let l = self.flows[&id].legs[i].as_ref().unwrap();
        // (fd, cookie, kind, fin, upstream sent cookie, dst cookie, base(sent dst, recv, fwd, upstream))
        let mut hops: Vec<(RawFd, u64, LegKind, bool, Option<u64>, u64, (u64, u64, u64, u64))> = vec![];
        let dst_ck = self.leg_ck(id, j);
        match (i, self.flows[&id].park) {
            (0, Some(slot)) => {
                let pk = &self.parks[slot];
                let (bs, br, bf) = pk.base;
                hops.push((l.fd.as_raw_fd(), l.cookie, LegKind::Tcp, true, None, pk.p1ck, (bs, 0, 0, 0)));
                hops.push((pk.p2.as_raw_fd(), pk.p2ck, LegKind::Tcp, false, Some(pk.p1ck), dst_ck, (0, br, bf, bs)));
            }
            _ => hops.push((l.fd.as_raw_fd(), l.cookie, l.kind, true, None, dst_ck, (0, 0, 0, 0))),
        }
        let mut all = true;
        let mut detail = vec![];
        for (fd, ck, kind, fin, upstream, dck, (b_sent, b_recv, b_fwd, b_up)) in hops {
            let fwd = self.percpu("FWD", ck).saturating_sub(b_fwd);
            let sent = self.percpu("SENT", dck).saturating_sub(b_sent);
            let q = inq(fd);
            let recv = if kind == LegKind::Tcp {
                tcp_bytes_received(fd).map(|r| r.saturating_sub(fin as u64).saturating_sub(b_recv))
            } else {
                None
            };
            let up = upstream.map(|u| self.percpu("SENT", u).saturating_sub(b_up));
            let q_ok = if kind == LegKind::Tcp { true } else { q == 0 };
            let ok = q_ok && recv.map_or(true, |r| r == fwd) && up.map_or(true, |u| recv == Some(u)) && sent >= fwd;
            all &= ok;
            detail.push(json!({"fwd":fwd,"sent":sent,"inq":q,"recv":recv,"upstream_sent":up}));
        }
        (all, json!(detail))
    }

    fn drain_tick(&mut self) {
        let ids: Vec<u32> = self.flows.iter().filter(|(_, f)| f.drain.iter().any(|d| d.is_some())).map(|(k, _)| *k).collect();
        for id in ids {
            for i in 0..2 {
                let Some(start) = self.flows.get(&id).and_then(|f| f.drain[i]) else { continue };
                let j = 1 - i;
                let (drained, detail) = self.hop_drained(id, i);
                let timed_out = start.elapsed() > Duration::from_secs(10);
                if drained || timed_out {
                    if timed_out {
                        self.bump("k2_drain_timeouts", 1);
                        self.ev(json!({"ev":"k2_drain_timeout","flow":id,"leg":i,"hops":detail}));
                    } else {
                        self.bump("k2_drained_shutdowns", 1);
                    }
                    let dfd = self.leg_fd(id, j);
                    unsafe { libc::shutdown(dfd, libc::SHUT_WR) };
                    let f = self.flows.get_mut(&id).unwrap();
                    f.drain[i] = None;
                    f.legs[j].as_mut().unwrap().shut = true;
                    let all = f.legs.iter().all(|l| l.as_ref().map_or(true, |l| l.eof && l.shut));
                    if all {
                        self.close_flow(id, false, "both_directions_done");
                        break;
                    }
                }
            }
        }
    }

    fn write_stats(&mut self) {
        let mut v = serde_json::Map::new();
        for (k, n) in &self.stat {
            v.insert(k.to_string(), json!(n));
        }
        v.insert("counters".into(), json!(self.counters()));
        v.insert("flows_open".into(), json!(self.flows.len()));
        v.insert("t_ms".into(), json!(now_ms()));
        let tmp = format!("{}.tmp", self.c.stats);
        std::fs::write(&tmp, serde_json::Value::Object(v).to_string()).unwrap();
        std::fs::rename(&tmp, &self.c.stats).unwrap();
    }
}

// ==================================================================== setup
pub fn run(args: &[String]) {
    let c = parse_cfg(args);
    let mut bpf = EbpfLoader::new().load_file(&c.obj).expect("load ELF");
    let log = File::create(&c.log).unwrap();
    let mut loaded = vec![];
    // Programs common to both sides.
    {
        let vfd = SockHash::<_, u64>::try_from(bpf.map("VERD").unwrap()).unwrap().fd().try_clone().unwrap();
        let p: &mut SkSkb = bpf.program_mut("verdict").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&vfd).unwrap();
        loaded.push(prog_info("verdict", p.info().unwrap()));
        let sfd = SockHash::<_, u64>::try_from(bpf.map("STRP").unwrap()).unwrap().fd().try_clone().unwrap();
        let p: &mut SkSkb = bpf.program_mut("strp_parse").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&sfd).unwrap();
        loaded.push(prog_info("strp_parse", p.info().unwrap()));
        let p: &mut SkSkb = bpf.program_mut("strp_verdict").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&sfd).unwrap();
        loaded.push(prog_info("strp_verdict", p.info().unwrap()));
        let p: &mut SchedClassifier = bpf.program_mut("lo_decode").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach("lo", TcAttachType::Egress).unwrap();
        loaded.push(prog_info("lo_decode(tcx lo egress)", p.info().unwrap()));
        let btf = Btf::from_sys_fs().unwrap();
        let p: &mut FExit = bpf.program_mut("sent").unwrap().try_into().unwrap();
        match p.load("skb_send_sock", &btf).and_then(|_| p.attach().map(|_| ())) {
            Ok(()) => loaded.push(prog_info("sent(fexit skb_send_sock)", p.info().unwrap())),
            Err(e) => loaded.push(json!({"name":"sent","error":format!("{e:?}")})),
        }
    }
    if c.role == "guest" {
        let cg = File::open("/sys/fs/cgroup").unwrap();
        for name in ["connect4", "sendmsg4", "recvmsg4", "getpeername4"] {
            let p: &mut CgroupSockAddr = bpf.program_mut(name).unwrap().try_into().unwrap();
            p.load().unwrap();
            p.attach(&cg, CgroupAttachMode::Single).unwrap();
            loaded.push(prog_info(name, p.info().unwrap()));
        }
        let p: &mut CgroupSock = bpf.program_mut("sock_release").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&cg, CgroupAttachMode::Single).unwrap();
        loaded.push(prog_info("sock_release", p.info().unwrap()));
        let p: &mut SockOps = bpf.program_mut("flow_sockops").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&cg, CgroupAttachMode::Single).unwrap();
        loaded.push(prog_info("sockops(root cgroup)", p.info().unwrap()));
        let mut a: Array<_, u32> = Array::try_from(bpf.map_mut("CFG").unwrap()).unwrap();
        let vals = [
            u32::from_ne_bytes(c.w.octets()),
            c.tcp_intake as u32,
            c.udp_base as u32,
            c.udp_slots,
            u32::from_ne_bytes(c.dns.ip().octets()),
            c.dns.port() as u32,
            1,
            c.intake_park as u32,
        ];
        for (i, v) in vals.iter().enumerate() {
            a.set(i as u32, v, 0).unwrap();
        }
        let mut q: Queue<_, u32> = Queue::try_from(bpf.map_mut("FREE").unwrap()).unwrap();
        for s in 0..c.udp_slots {
            q.push(s, 0).unwrap();
        }
    }
    if c.role == "host" {
        // sock_ops only for the owner's own sockets: a dedicated cgroup holding just this
        // process (intake listener children and armed destination legs). Never the root.
        let dir = format!("/sys/fs/cgroup/gvc-spike-{}", std::process::id());
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(format!("{dir}/cgroup.procs"), std::process::id().to_string()).unwrap();
        let cg = File::open(&dir).unwrap();
        let p: &mut SockOps = bpf.program_mut("flow_sockops").unwrap().try_into().unwrap();
        p.load().unwrap();
        p.attach(&cg, CgroupAttachMode::Single).unwrap();
        loaded.push(prog_info(&format!("sockops({dir})"), p.info().unwrap()));
        let mut a: Array<_, u32> = Array::try_from(bpf.map_mut("CFG").unwrap()).unwrap();
        a.set(7, c.intake_park as u32, 0).unwrap();
    }
    let ep = Epoll::new();
    let mut o = Owner {
        c,
        bpf,
        ep,
        log,
        ctl: StdMap::new(),
        ctl_in: StdMap::new(),
        flows: BTreeMap::new(),
        fdmap: StdMap::new(),
        listeners: StdMap::new(),
        next_flow: 1,
        dg_pending: StdMap::new(),
        host_dg: StdMap::new(),
        hostdg_fd: StdMap::new(),
        slots: vec![],
        p2fd: StdMap::new(),
        assocfd: StdMap::new(),
        flow_slot: StdMap::new(),
        stat: BTreeMap::new(),
        last_stats: Instant::now(),
        pending_inband: StdMap::new(),
        parks: vec![],
    };
    o.ev(json!({"ev":"programs_loaded","role":o.c.role,"programs":loaded,"ack_oob":o.c.ack_oob,"flush":o.c.flush,"udp_park":o.c.udp_park,"intake_park":o.c.intake_park,"arm":o.c.arm,"pid":std::process::id()}));
    make_park_pool(&mut o);
    if o.c.role == "guest" {
        guest_setup(&mut o);
    } else {
        host_setup(&mut o);
    }
    o.write_stats();
    println!("OWNER_READY role={} pid={}", o.c.role, std::process::id());
    loop {
        let evs = o.ep.wait(1);
        for (fd, ev) in evs {
            dispatch(&mut o, fd, ev);
        }
        o.drain_tick();
        if o.c.role == "guest" {
            guest_released_tick(&mut o);
        }
        if o.last_stats.elapsed() > Duration::from_millis(500) {
            o.write_stats();
            o.last_stats = Instant::now();
        }
    }
}

fn prog_info(name: &str, i: aya::programs::ProgramInfo) -> serde_json::Value {
    json!({"name":name,"id":i.id(),"tag":format!("{:016x}",i.tag()),"verified_insns":i.verified_instruction_count()})
}

fn listen_tcp(a: SocketAddrV4, transparent: bool) -> OwnedFd {
    let fd = socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK).unwrap();
    setopt_i32(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1).unwrap();
    if transparent {
        setopt_i32(fd.as_raw_fd(), libc::SOL_IP, libc::IP_TRANSPARENT, 1).unwrap();
    }
    bind4(fd.as_raw_fd(), a).unwrap();
    cvt(unsafe { libc::listen(fd.as_raw_fd(), 4096) }).unwrap();
    fd
}

/// Kernel TCP parking cells: loopback P1->P2 pairs. P1 is a redirect target (SOCKS); P2
/// stays an ordinary socket (its receive queue is the parking space) until its flow pairs.
fn make_park_pool(o: &mut Owner) {
    let pl = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    bind4(pl.as_raw_fd(), "127.0.0.1:0".parse().unwrap()).unwrap();
    cvt(unsafe { libc::listen(pl.as_raw_fd(), 4096) }).unwrap();
    let paddr = local4(pl.as_raw_fd()).unwrap();
    for slot in 0..o.c.park_pool {
        let p1 = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
        connect4(p1.as_raw_fd(), paddr).unwrap();
        let p2 = accept(pl.as_raw_fd()).unwrap();
        // large receive buffer: the cell must hold everything a client writes before pairing
        setopt_i32(p2.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 4 << 20).unwrap();
        let p1ck = o.add_sock("SOCKS", p1.as_raw_fd());
        let p2ck = cookie(p2.as_raw_fd());
        {
            let mut a: Array<_, u64> = Array::try_from(o.bpf.map_mut("TCPPARK").unwrap()).unwrap();
            a.set(slot, p1ck, 0).unwrap();
            let mut q: Queue<_, u32> = Queue::try_from(o.bpf.map_mut("TCPFREE").unwrap()).unwrap();
            q.push(slot, 0).unwrap();
        }
        o.parks.push(Park { p1, p2, p1ck, p2ck, base: (0, 0, 0) });
    }
    o.ev(json!({"ev":"park_pool","slots":o.c.park_pool}));
}

fn mark_intake(o: &mut Owner, port: u16) {
    let mut m: HashMap<_, u32, u32> = HashMap::try_from(o.bpf.map_mut("INTAKE").unwrap()).unwrap();
    m.insert(port as u32, 1, 0).unwrap();
}

fn add_listener(o: &mut Owner, fd: OwnedFd, what: &'static str, port: u16) {
    let raw = fd.as_raw_fd();
    o.ep.add(raw, libc::EPOLLIN as u32);
    o.listeners.insert(raw, (fd, what, port));
}

fn guest_setup(o: &mut Owner) {
    // control channel to the host owner (beacon-like session keyed by our CID on the host)
    let ctl = vsock_connect(2, CTL_PORT, libc::SOCK_STREAM).expect("ctl connect");
    let mut hello = [0u8; 8];
    hello[0] = M_HELLO;
    let _ = write_all_ctl(ctl.as_raw_fd(), &hello);
    o.ep.add(ctl.as_raw_fd(), libc::EPOLLIN as u32);
    o.ctl_in.insert(ctl.as_raw_fd(), (2, vec![]));
    o.ctl.insert(2, ctl);
    let l = listen_tcp(SocketAddrV4::new(o.c.w, o.c.tcp_intake), false);
    add_listener(o, l, "tcp_intake", o.c.tcp_intake);
    mark_intake(o, o.c.tcp_intake);
    let vl = vsock_listen(u32::MAX, GUEST_ACCEPT_PORT, libc::SOCK_STREAM | libc::SOCK_NONBLOCK).unwrap();
    add_listener(o, vl, "vsock_accept", 0);
    // UDP slot pool with TCP parking cells (strparser) on loopback.
    let pl = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    bind4(pl.as_raw_fd(), "127.0.0.1:0".parse().unwrap()).unwrap();
    cvt(unsafe { libc::listen(pl.as_raw_fd(), 1024) }).unwrap();
    let paddr = local4(pl.as_raw_fd()).unwrap();
    for s in 0..o.c.udp_slots {
        let up = socket(libc::AF_INET, libc::SOCK_DGRAM).unwrap();
        setopt_i32(up.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1).unwrap();
        setopt_i32(up.as_raw_fd(), libc::SOL_SOCKET, libc::SO_NO_CHECK, 1).unwrap();
        bind4(up.as_raw_fd(), SocketAddrV4::new(o.c.w, o.c.udp_base + s as u16)).unwrap();
        let p1 = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
        connect4(p1.as_raw_fd(), paddr).unwrap();
        let p2 = accept(pl.as_raw_fd()).unwrap();
        let p1ck = o.add_sock("SOCKS", p1.as_raw_fd());
        // Reply cell: guest vsock RX delivers a host datagram frame split into <=4 KiB
        // skbs (guest RX buffers); a TCP strparser reassembles exactly one frame.
        let p3 = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
        connect4(p3.as_raw_fd(), paddr).unwrap();
        let p4 = accept(pl.as_raw_fd()).unwrap();
        setopt_i32(p4.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 4 << 20).unwrap();
        o.add_sock("SOCKS", p3.as_raw_fd());
        let upck_r = cookie(up.as_raw_fd());
        let p4ck = cookie(p4.as_raw_fd());
        o.route(p4ck, upck_r, 2);
        o.add_sock("STRP", p4.as_raw_fd());
        if o.c.udp_park {
            let upck = o.add_sock("SOCKS", up.as_raw_fd());
            o.route(upck, p1ck, 1);
            o.add_sock("VERD", up.as_raw_fd());
            o.ep.add(p2.as_raw_fd(), libc::EPOLLIN as u32);
            o.p2fd.insert(p2.as_raw_fd(), s as usize);
        } else {
            o.ep.add(up.as_raw_fd(), libc::EPOLLIN as u32);
            o.p2fd.insert(up.as_raw_fd(), s as usize);
        }
        o.slots.push(GuestSlot { up, p1, p2, p3, p4, assoc: None, active: false, tuple: None, released: false, installed_naive: false, retry_at: None });
    }
    o.ev(json!({"ev":"guest_ready","w":o.c.w.to_string(),"slots":o.c.udp_slots}));
}

fn host_setup(o: &mut Owner) {
    for (port, ty, what) in [(CTL_PORT, libc::SOCK_STREAM, "ctl"), (FLOW_PORT, libc::SOCK_STREAM, "flow"), (SEQ_PORT, libc::SOCK_SEQPACKET, "seq")] {
        let l = vsock_listen(u32::MAX, port, ty | libc::SOCK_NONBLOCK).unwrap(); // VMADDR_CID_ANY; peer CID checked on accept
        add_listener(o, l, what, 0);
    }
    for (a, g) in o.c.intakes.clone() {
        let l = listen_tcp(a, false);
        add_listener(o, l, "intake", g);
        mark_intake(o, a.port());
    }
    o.ev(json!({"ev":"host_ready","cid":o.c.cid,"intakes":o.c.intakes.iter().map(|(a,g)|format!("{a}={g}")).collect::<Vec<_>>()}));
}

fn dispatch(o: &mut Owner, fd: RawFd, ev: u32) {
    if let Some((_, what, port)) = o.listeners.get(&fd).map(|(f, w, p)| (f.as_raw_fd(), *w, *p)) {
        loop {
            let Ok(n) = accept(fd) else { break };
            match what {
                "tcp_intake" => guest_intake(o, n),
                "vsock_accept" => guest_tcp_accept(o, n),
                "ctl" => {
                    let (cid, _) = vsock_peer(n.as_raw_fd()).unwrap();
                    o.ep.add(n.as_raw_fd(), libc::EPOLLIN as u32);
                    o.ctl_in.insert(n.as_raw_fd(), (cid, vec![]));
                    o.ctl.insert(cid, n);
                }
                "flow" | "seq" => host_flow_accept(o, n, what == "seq"),
                "intake" => host_intake(o, n, port),
                _ => {}
            }
        }
        return;
    }
    if o.ctl_in.contains_key(&fd) {
        ctl_read(o, fd);
        return;
    }
    if let Some(id) = o.pending_inband.get(&fd).copied() {
        inband_ack(o, fd, id);
        return;
    }
    if let Some(s) = o.p2fd.get(&fd).copied() {
        guest_slot_ready(o, s);
        return;
    }
    if let Some(s) = o.assocfd.get(&fd).copied() {
        if ev & (libc::EPOLLHUP | libc::EPOLLRDHUP | libc::EPOLLERR) as u32 != 0 {
            guest_slot_teardown(o, s, "assoc_hup");
        }
        return;
    }
    if let Some(id) = o.hostdg_fd.get(&fd).copied() {
        if ev & (libc::EPOLLHUP | libc::EPOLLRDHUP | libc::EPOLLERR) as u32 != 0 {
            host_dg_teardown(o, id);
        }
        return;
    }
    o.on_leg_event(fd, ev);
}

fn ctl_read(o: &mut Owner, fd: RawFd) {
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
    if n <= 0 {
        if n == 0 {
            o.ev(json!({"ev":"ctl_closed"}));
            o.ep.del(fd);
            o.ctl_in.remove(&fd);
            // Fail closed: the peer owner is gone; abort every flow it was part of.
            let ids: Vec<u32> = o.flows.keys().copied().collect();
            o.bump("aborted_on_ctl_loss", ids.len() as u64);
            for id in ids {
                o.close_flow(id, true, "ctl_lost");
            }
        }
        return;
    }
    o.bump("ctl_bytes_read", n as u64);
    let (cid, mut acc) = o.ctl_in.remove(&fd).unwrap();
    acc.extend_from_slice(&buf[..n as usize]);
    while acc.len() >= 8 {
        let m: Vec<u8> = acc.drain(..8).collect();
        let flow = u32::from_le_bytes(m[4..8].try_into().unwrap());
        match m[0] {
            M_PAIRED => on_paired(o, flow),
            M_REFUSED => {
                o.bump("refused", 1);
                o.ev(json!({"ev":"refused","flow":flow}));
                if let Some(s) = o.flow_slot.get(&flow).copied() {
                    guest_slot_teardown(o, s, "refused");
                } else {
                    o.close_flow(flow, true, "refused");
                }
            }
            M_ABORT => {
                o.bump("abort_received", 1);
                o.close_flow(flow, true, "peer_abort");
            }
            M_HELLO => o.ev(json!({"ev":"hello","cid":cid})),
            _ => {}
        }
    }
    o.ctl_in.insert(fd, (cid, acc));
}

fn on_paired(o: &mut Owner, id: u32) {
    o.bump("paired_received", 1);
    if let Some(s) = o.flow_slot.get(&id).copied() {
        guest_slot_paired(o, s, id);
        return;
    }
    let Some(f) = o.flows.get(&id) else { return };
    if f.state != FState::AwaitAck {
        return;
    }
    // Opener: the vsock leg was installed right after connect(); now start the inet leg.
    o.activate_inet(id);
}

/// ADR-0158 in-band ack comparison: the opener reads exactly 4 bytes, then installs.
fn inband_ack(o: &mut Owner, fd: RawFd, id: u32) {
    let mut b = [0u8; 4];
    o.pending_inband.remove(&fd);
    o.ep.del(fd);
    if read_exact_ctl(fd, &mut b, Duration::from_secs(5)).is_err() || &b != b"PAIR" {
        o.ev(json!({"ev":"inband_ack_bad","flow":id,"got":String::from_utf8_lossy(&b)}));
        o.close_flow(id, true, "inband_ack_bad");
        return;
    }
    o.bump("inband_ack_read", 4);
    o.install_vsock(id);
    o.activate_inet(id);
}

// ==================================================================== guest TCP egress (opener)
fn park_of(o: &Owner, ck: u64) -> Option<usize> {
    let m: HashMap<_, u64, u32> = HashMap::try_from(o.bpf.map("PARK_OF").unwrap()).unwrap();
    m.get(&ck, 0).ok().map(|s| s as usize)
}

/// Opener for an intake child (guest TCP egress, host TCP inbound). The intake child was
/// installed at PASSIVE_ESTABLISHED (sock_ops) and parks its bytes; in late mode it is
/// installed by the owner after Paired (ADR-0158 shape).
fn opener_setup(o: &mut Owner, id: u32, kind: u8, s: OwnedFd, vcid: u32, vport: u32, req: [u8; 16]) {
    let ck = cookie(s.as_raw_fd());
    let park = if o.c.intake_park { park_of(o, ck) } else { None };
    o.new_flow(id, kind, FState::AwaitAck);
    o.set_leg(id, 0, s, LegKind::Tcp);
    o.flows.get_mut(&id).unwrap().park = park;
    if o.c.intake_park && park.is_none() {
        o.bump("intake_not_parked", 1);
    }
    if park.is_some() && o.c.flush {
        // The child was installed at PASSIVE_ESTABLISHED, but until accept() it has no
        // struct socket and sk_psock_verdict_data_ready() returns early, swallowing the
        // wakeup: bytes (and a FIN) that arrived before accept() sit in the receive queue.
        // Re-arm data_ready now (tcp_set_rcvlowat -> tcp_data_ready). No payload read.
        setopt_i32(o.leg_fd(id, 0), libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
        o.bump("post_accept_rearm", 1);
    }
    if park.is_none() {
        // late/unparked: the leg must be a SOCKS member before the vsock leg can target it
        let before = o.stat.get("sockhash_insert_errors").copied().unwrap_or(0);
        o.add_sock("SOCKS", o.leg_fd(id, 0));
        if o.stat.get("sockhash_insert_errors").copied().unwrap_or(0) != before {
            o.bump("late_install_impossible", 1);
            o.close_flow(id, true, "intake_not_installable");
            return;
        }
    }
    let v = match vsock_connect(vcid, vport, libc::SOCK_STREAM) {
        Ok(v) => v,
        Err(e) => {
            o.ev(json!({"ev":"vsock_connect_failed","err":e.to_string()}));
            o.close_flow(id, true, "vsock_connect_failed");
            return;
        }
    };
    let vraw = v.as_raw_fd();
    o.set_leg(id, 1, v, LegKind::Vsock);
    if o.c.ack_oob {
        // Install the vsock leg before the peer owner can possibly write on it.
        o.install_vsock(id);
    } else {
        o.pending_inband.insert(vraw, id);
        o.ep.add(vraw, libc::EPOLLIN as u32);
    }
    let _ = write_all_ctl(vraw, &req);
    o.bump("requests_written", 1);
}

fn guest_intake(o: &mut Owner, s: OwnedFd) {
    let app = peer4(s.as_raw_fd()).unwrap();
    let dst = {
        let m: HashMap<_, u32, Dst> = HashMap::try_from(o.bpf.map("FLOWP").unwrap()).unwrap();
        m.get(&(app.port() as u32), 0).ok()
    };
    let Some(dst) = dst else {
        o.bump("intake_no_flow", 1);
        o.ev(json!({"ev":"intake_without_flow","app":app.to_string()}));
        linger0_close(s);
        return;
    };
    let dst = SocketAddrV4::new(Ipv4Addr::from(dst.ip.to_ne_bytes()), dst.port as u16);
    let id = o.next_flow;
    o.next_flow += 2;
    let req = Owner::request(K_TCP_CONNECT, *dst.ip(), dst.port(), id, 0);
    opener_setup(o, id, K_TCP_CONNECT, s, 2, FLOW_PORT, req);
    o.bump("tcp_connect_flows", 1);
    if id < o.c.verbose_flows {
        let park = o.flows.get(&id).and_then(|f| f.park);
        o.ev(json!({"ev":"tcp_connect","flow":id,"app":app.to_string(),"dst":dst.to_string(),"park":park}));
    }
}

/// Acceptor-side active leg: connect `c` to `to`. With --arm yes the socket is installed
/// by sock_ops at ACTIVE_ESTABLISHED (route preset to the vsock leg), so bytes and even a
/// FIN the peer sends right away are covered. Otherwise install late (+flush).
fn acceptor_connect(o: &mut Owner, id: u32, c: OwnedFd, to: SocketAddrV4, t: Duration) -> std::io::Result<()> {
    let v_ck = o.leg_ck(id, 1);
    if o.c.arm {
        o.arm_active(c.as_raw_fd(), v_ck);
    }
    connect4_timeout(c.as_raw_fd(), to, t)?;
    o.set_leg(id, 0, c, LegKind::Tcp);
    Ok(())
}

// ==================================================================== guest TCP inbound (acceptor)
fn guest_tcp_accept(o: &mut Owner, v: OwnedFd) {
    let mut req = [0u8; 16];
    if read_exact_ctl(v.as_raw_fd(), &mut req, Duration::from_secs(5)).is_err() || req[0] != K_TCP_ACCEPT {
        o.ev(json!({"ev":"bad_accept_request"}));
        return;
    }
    let client = SocketAddrV4::new(Ipv4Addr::new(req[4], req[5], req[6], req[7]), u16::from_be_bytes([req[2], req[3]]));
    let flow = u32::from_le_bytes(req[8..12].try_into().unwrap());
    let gport = u16::from_be_bytes([req[12], req[13]]);
    let vraw = v.as_raw_fd();
    o.new_flow(flow, K_TCP_ACCEPT, FState::AwaitAck);
    o.set_leg(flow, 1, v, LegKind::Vsock);
    o.add_sock("SOCKS", vraw);
    let c = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    // D15: the application sees the real client address as its peer.
    setopt_i32(c.as_raw_fd(), libc::SOL_IP, libc::IP_TRANSPARENT, 1).unwrap();
    setopt_i32(c.as_raw_fd(), libc::SOL_SOCKET, libc::SO_MARK, MARK).unwrap();
    setopt_i32(c.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1).unwrap();
    let r = bind4(c.as_raw_fd(), client).and_then(|_| acceptor_connect(o, flow, c, SocketAddrV4::new(o.c.w, gport), Duration::from_secs(3)));
    if let Err(e) = r {
        o.ev(json!({"ev":"tcp_accept_refused","flow":flow,"client":client.to_string(),"gport":gport,"err":e.to_string()}));
        if o.c.ack_oob {
            o.send_ctl(2, M_REFUSED, flow);
        } else {
            let _ = write_all_ctl(vraw, b"RFSD");
        }
        o.close_flow(flow, false, "refused");
        return;
    }
    if !o.c.ack_oob {
        // ADR-0158 D18: the guest acceptor writes Paired BEFORE installing.
        let _ = write_all_ctl(vraw, b"PAIR");
        o.bump("inband_ack_written", 4);
    }
    if !o.c.arm && !o.late_install_inet(flow, true) {
        o.close_flow(flow, true, "accept_leg_not_installable");
        return;
    }
    let (v_ck, c_ck) = (o.leg_ck(flow, 1), o.leg_ck(flow, 0));
    o.route(v_ck, c_ck, 0);
    o.add_sock("VERD", vraw);
    o.flows.get_mut(&flow).unwrap().state = FState::Active;
    o.watch(flow);
    o.bump("flows_active_total", 1);
    if o.c.ack_oob {
        o.send_ctl(2, M_PAIRED, flow);
    }
    o.bump("tcp_accept_flows", 1);
    if flow < o.c.verbose_flows {
        o.ev(json!({"ev":"tcp_accept","flow":flow,"client":client.to_string(),"gport":gport}));
    }
}

// ==================================================================== guest UDP slots
fn guest_slot_ready(o: &mut Owner, s: usize) {
    if o.slots[s].assoc.is_some() {
        return;
    }
    let info = {
        let a: Array<_, Slot> = Array::try_from(o.bpf.map("SLOT").unwrap()).unwrap();
        a.get(&(s as u32), 0).unwrap()
    };
    if info.app_port == 0 {
        o.bump("slot_ready_without_app_port", 1);
        return;
    }
    let raw_trigger = if o.c.udp_park { o.slots[s].p2.as_raw_fd() } else { o.slots[s].up.as_raw_fd() };
    o.ep.del(raw_trigger);
    let up = o.slots[s].up.as_raw_fd();
    let app = SocketAddrV4::new(o.c.w, info.app_port as u16);
    connect4(up, app).unwrap();
    let dst = SocketAddrV4::new(Ipv4Addr::from(info.ip.to_ne_bytes()), info.port as u16);
    let id = o.next_flow;
    o.next_flow += 2;
    let (t, q) = match (vsock_connect(2, FLOW_PORT, libc::SOCK_STREAM), vsock_connect(2, SEQ_PORT, libc::SOCK_SEQPACKET)) {
        (Ok(t), Ok(q)) => (t, q),
        (a, b) => {
            // Host owner unavailable: the datagram stays parked in the kernel cell; retry later.
            let e = a.err().or(b.err()).map(|e| e.to_string());
            o.bump("udp_assoc_connect_failed", 1);
            o.ev(json!({"ev":"udp_assoc_connect_failed","slot":s,"err":e}));
            let _ = disconnect(up);
            o.slots[s].retry_at = Some(Instant::now() + Duration::from_secs(1));
            return;
        }
    };
    let up_ck = cookie(up);
    if !o.c.udp_park {
        o.add_sock("SOCKS", up); // park mode: already a member since boot
    }
    let t_ck = o.add_sock("SOCKS", t.as_raw_fd());
    let _ = up_ck;
    let p3ck = cookie(o.slots[s].p3.as_raw_fd());
    o.route(t_ck, p3ck, 0);
    o.add_sock("VERD", t.as_raw_fd());
    let _ = write_all_ctl(t.as_raw_fd(), &Owner::request(K_DG_STREAM, *dst.ip(), dst.port(), id, 0));
    o.add_sock("SOCKS", q.as_raw_fd());
    let _ = write_all_ctl(q.as_raw_fd(), &Owner::request(K_DG_SEQ, *dst.ip(), dst.port(), id, 0));
    o.bump("requests_written", 2);
    let tuple = Tuple {
        src: u32::from_ne_bytes(o.c.w.octets()),
        dst: u32::from_ne_bytes(o.c.w.octets()),
        sport: (o.c.udp_base + s as u16).to_be(),
        dport: (info.app_port as u16).to_be(),
    };
    o.tuple_set(tuple, Some(2));
    for fd in [t.as_raw_fd(), q.as_raw_fd()] {
        o.ep.add(fd, (libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLET) as u32);
        o.assocfd.insert(fd, s);
    }
    let sl = &mut o.slots[s];
    sl.assoc = Some((id, t, q));
    sl.tuple = Some(tuple);
    o.flow_slot.insert(id, s);
    o.bump("udp_associations", 1);
    o.ev(json!({"ev":"udp_associate","slot":s,"flow":id,"app":app.to_string(),"dst":dst.to_string(),"queued_on_trigger":inq(raw_trigger)}));
}

fn guest_slot_paired(o: &mut Owner, s: usize, id: u32) {
    let Some((aid, _, q)) = o.slots[s].assoc.as_ref() else { return };
    if *aid != id {
        return;
    }
    let q_ck = cookie(q.as_raw_fd());
    if o.c.udp_park {
        let p2 = o.slots[s].p2.as_raw_fd();
        let p2ck = cookie(p2);
        o.route(p2ck, q_ck, 4);
        o.add_sock("STRP", p2);
        setopt_i32(p2, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
        o.bump("park_flush", 1);
    } else {
        // naive ADR-0158 intake: install the UDP intake now (first datagram already queued)
        let up = o.slots[s].up.as_raw_fd();
        let upck = cookie(up);
        o.route(upck, q_ck, 1);
        o.add_sock("VERD", up);
        let _ = setopt_i32(up, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1);
        o.slots[s].installed_naive = true;
    }
    o.slots[s].active = true;
    o.ev(json!({"ev":"udp_paired","slot":s,"flow":id}));
}

fn guest_slot_teardown(o: &mut Owner, s: usize, why: &str) {
    let Some((id, t, q)) = o.slots[s].assoc.take() else { return };
    for fd in [t.as_raw_fd(), q.as_raw_fd()] {
        o.ep.del(fd);
        o.assocfd.remove(&fd);
        o.unroute(cookie(fd));
        o.forget_counters(cookie(fd));
    }
    drop(t);
    drop(q);
    o.flow_slot.remove(&id);
    if let Some(tu) = o.slots[s].tuple.take() {
        o.tuple_set(tu, None);
    }
    let up = o.slots[s].up.as_raw_fd();
    let _ = disconnect(up);
    if o.c.udp_park {
        let p2 = o.slots[s].p2.as_raw_fd();
        let ck = cookie(p2);
        o.unroute(ck);
        o.del_sock("STRP", ck);
        o.ep.add(p2, libc::EPOLLIN as u32);
    } else {
        let ck = cookie(up);
        o.unroute(ck);
        o.del_sock("VERD", ck);
        o.ep.add(up, libc::EPOLLIN as u32);
    }
    o.slots[s].active = false;
    o.bump("udp_teardowns", 1);
    o.ev(json!({"ev":"udp_teardown","slot":s,"flow":id,"why":why}));
    if o.slots[s].released {
        o.slots[s].released = false;
        let mut f: Queue<_, u32> = Queue::try_from(o.bpf.map_mut("FREE").unwrap()).unwrap();
        let _ = f.push(s as u32, 0);
    }
}

fn guest_released_tick(o: &mut Owner) {
    let due: Vec<usize> = (0..o.slots.len()).filter(|&s| o.slots[s].retry_at.map_or(false, |t| t <= Instant::now())).collect();
    for s in due {
        o.slots[s].retry_at = None;
        guest_slot_ready(o, s);
    }
    let mut rel = vec![];
    {
        let mut q: Queue<_, u32> = Queue::try_from(o.bpf.map_mut("RELEASED").unwrap()).unwrap();
        while let Ok(s) = q.pop(0) {
            rel.push(s as usize);
        }
    }
    for s in rel {
        o.slots[s].released = true;
        o.bump("udp_slot_released", 1);
        if o.slots[s].assoc.is_some() {
            guest_slot_teardown(o, s, "app_socket_released");
        } else {
            o.slots[s].released = false;
            let _ = disconnect(o.slots[s].up.as_raw_fd());
            let mut f: Queue<_, u32> = Queue::try_from(o.bpf.map_mut("FREE").unwrap()).unwrap();
            let _ = f.push(s as u32, 0);
        }
    }
}

// ==================================================================== host side
fn host_flow_accept(o: &mut Owner, v: OwnedFd, seq: bool) {
    let (cid, _) = vsock_peer(v.as_raw_fd()).unwrap();
    if cid != o.c.cid {
        o.ev(json!({"ev":"foreign_cid","cid":cid}));
        return;
    }
    let mut req = [0u8; 16];
    if seq {
        let mut big = [0u8; 64];
        set_timeouts(v.as_raw_fd(), Duration::from_secs(5));
        let n = unsafe { libc::recv(v.as_raw_fd(), big.as_mut_ptr() as *mut _, 16, libc::MSG_TRUNC) };
        set_timeouts(v.as_raw_fd(), Duration::ZERO);
        if n != 16 {
            o.ev(json!({"ev":"bad_seq_request","n":n}));
            return;
        }
        req.copy_from_slice(&big[..16]);
    } else if read_exact_ctl(v.as_raw_fd(), &mut req, Duration::from_secs(5)).is_err() {
        o.ev(json!({"ev":"bad_request"}));
        return;
    }
    o.bump("request_bytes_read", 16);
    let dst = SocketAddrV4::new(Ipv4Addr::new(req[4], req[5], req[6], req[7]), u16::from_be_bytes([req[2], req[3]]));
    let flow = u32::from_le_bytes(req[8..12].try_into().unwrap());
    match req[0] {
        K_TCP_CONNECT => host_tcp_connect(o, v, dst, flow),
        K_DG_STREAM | K_DG_SEQ => {
            let p = o.dg_pending.entry(flow).or_insert(DgPending { stream: None, seq: None, dst, since: Instant::now() });
            if req[0] == K_DG_STREAM {
                p.stream = Some(v);
            } else {
                p.seq = Some(v);
            }
            if p.stream.is_some() && p.seq.is_some() {
                let p = o.dg_pending.remove(&flow).unwrap();
                host_dg_install(o, flow, p);
            }
        }
        k => o.ev(json!({"ev":"unknown_kind","kind":k})),
    }
}

fn host_tcp_connect(o: &mut Owner, v: OwnedFd, dst: SocketAddrV4, flow: u32) {
    let vraw = v.as_raw_fd();
    o.new_flow(flow, K_TCP_CONNECT, FState::AwaitAck);
    o.set_leg(flow, 1, v, LegKind::Vsock);
    o.add_sock("SOCKS", vraw);
    let d = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    if let Err(e) = acceptor_connect(o, flow, d, dst, Duration::from_secs(5)) {
        o.ev(json!({"ev":"dest_connect_failed","flow":flow,"dst":dst.to_string(),"err":e.to_string()}));
        if o.c.ack_oob {
            o.send_ctl(o.c.cid, M_REFUSED, flow);
        } else {
            let _ = write_all_ctl(vraw, b"RFSD");
        }
        o.close_flow(flow, false, "dest_refused");
        return;
    }
    // ADR-0158: the host acceptor installs before Paired.
    let (v_ck, d_ck) = (o.leg_ck(flow, 1), o.leg_ck(flow, 0));
    o.route(v_ck, d_ck, 0);
    o.add_sock("VERD", vraw);
    if !o.c.arm && !o.late_install_inet(flow, o.c.ack_oob) {
        o.close_flow(flow, true, "dest_leg_not_installable");
        return;
    }
    if o.c.ack_oob {
        o.send_ctl(o.c.cid, M_PAIRED, flow);
    } else {
        let _ = write_all_ctl(vraw, b"PAIR");
        o.bump("inband_ack_written", 4);
        if !o.c.arm {
            // most favourable reading of D18 for a speaks-first destination: release bytes
            // queued on D before install only after Paired is in the stream
            setopt_i32(o.leg_fd(flow, 0), libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1).unwrap();
            o.bump("rcvlowat_rearm", 1);
        }
    }
    o.flows.get_mut(&flow).unwrap().state = FState::Active;
    o.watch(flow);
    o.bump("flows_active_total", 1);
    o.bump("tcp_connect_flows", 1);
    if flow < o.c.verbose_flows {
        o.ev(json!({"ev":"host_tcp_connect","flow":flow,"dst":dst.to_string()}));
    }
}

fn host_dg_install(o: &mut Owner, flow: u32, p: DgPending) {
    let h = socket(libc::AF_INET, libc::SOCK_DGRAM).unwrap();
    setopt_i32(h.as_raw_fd(), libc::SOL_SOCKET, libc::SO_NO_CHECK, 1).unwrap();
    if let Err(e) = connect4(h.as_raw_fd(), p.dst) {
        o.ev(json!({"ev":"dg_connect_failed","dst":p.dst.to_string(),"err":e.to_string()}));
        o.send_ctl(o.c.cid, M_REFUSED, flow);
        return;
    }
    let local = local4(h.as_raw_fd()).unwrap();
    let t = p.stream.unwrap();
    let s = p.seq.unwrap();
    let tuple = Tuple {
        src: u32::from_ne_bytes(local.ip().octets()),
        dst: u32::from_ne_bytes(p.dst.ip().octets()),
        sport: local.port().to_be(),
        dport: p.dst.port().to_be(),
    };
    o.tuple_set(tuple, Some(2));
    let t_ck = o.add_sock("SOCKS", t.as_raw_fd());
    let h_ck = o.add_sock("SOCKS", h.as_raw_fd());
    let s_ck = cookie(s.as_raw_fd());
    o.route(h_ck, t_ck, 1);
    o.add_sock("VERD", h.as_raw_fd());
    o.route(s_ck, h_ck, 2);
    o.add_sock("VERD", s.as_raw_fd());
    for fd in [t.as_raw_fd(), s.as_raw_fd()] {
        o.ep.add(fd, (libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLET) as u32);
        o.hostdg_fd.insert(fd, flow);
    }
    o.host_dg.insert(flow, HostDg { h, t, s, tuple });
    o.send_ctl(o.c.cid, M_PAIRED, flow);
    o.bump("udp_associations", 1);
    o.ev(json!({"ev":"host_dg_installed","flow":flow,"dst":p.dst.to_string(),"local":local.to_string(),"waited_ms":p.since.elapsed().as_millis() as u64}));
}

fn host_dg_teardown(o: &mut Owner, flow: u32) {
    let Some(d) = o.host_dg.remove(&flow) else { return };
    for fd in [d.t.as_raw_fd(), d.s.as_raw_fd(), d.h.as_raw_fd()] {
        o.ep.del(fd);
        o.hostdg_fd.remove(&fd);
        o.unroute(cookie(fd));
        o.forget_counters(cookie(fd));
    }
    o.tuple_set(d.tuple, None);
    o.bump("udp_teardowns", 1);
    o.ev(json!({"ev":"host_dg_teardown","flow":flow}));
}

fn host_intake(o: &mut Owner, a: OwnedFd, gport: u16) {
    let client = peer4(a.as_raw_fd()).unwrap();
    let local = local4(a.as_raw_fd()).unwrap();
    // Only host-local clients may use the spike intake (the metal host is public).
    if client.ip() != local.ip() && !client.ip().is_loopback() {
        o.bump("intake_foreign_rejected", 1);
        linger0_close(a);
        return;
    }
    let id = o.next_flow + 1; // host-opened flows are even
    o.next_flow += 2;
    let req = Owner::request(K_TCP_ACCEPT, *client.ip(), client.port(), id, gport);
    let cid = o.c.cid;
    opener_setup(o, id, K_TCP_ACCEPT, a, cid, GUEST_ACCEPT_PORT, req);
    o.bump("tcp_accept_flows", 1);
    if id < o.c.verbose_flows {
        let park = o.flows.get(&id).and_then(|f| f.park);
        o.ev(json!({"ev":"host_intake","flow":id,"client":client.to_string(),"gport":gport,"park":park}));
    }
}
