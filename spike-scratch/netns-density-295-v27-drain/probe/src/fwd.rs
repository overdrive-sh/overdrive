//! One forwarder role (guest or host stand-in). Single-threaded epoll owner; never reads or
//! writes payload on a paired socket. Completion of a half-close drain is either the owner's
//! own compare-and-swap (set-goal-then-check) or ONE BPF ringbuf record from the kernel.
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aya::maps::{MapData, RingBuf};
use serde_json::json;

use crate::bpfh::{Bpf, Rec, NONE, ROLE_SPLIT};
use crate::sys::*;

pub const VPORT: u32 = 7900;
const TOK_LST: u64 = 1 << 62;
const TOK_RING: u64 = (1 << 62) + 1;
const TOK_CHAN: u64 = (1 << 62) + 2;
const TOK_CTL: u64 = (1 << 62) + 3;
fn flow_tok(id: u32, which: u8) -> u64 {
    ((id as u64) << 1) | which as u64
}
pub const HDR_MAGIC: u32 = 0x4452_4e31; // "DRN1"

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArmMode {
    /// naive: shutdown the partner at EPOLLRDHUP (the premature half-close ADR-0160 rejects)
    None,
    /// R1: the owner sets the goal; kernel (fexit) or owner decides completion
    Owner,
    /// R1-alt: sock_ops STATE_CB arms the TCP-source goal in the kernel; vsock sources still owner-armed
    Kernel,
}

#[derive(Clone)]
pub struct RoleCfg {
    pub name: &'static str,
    pub arm: ArmMode,
    pub nflows: u32,
    pub intake_port: u16,
    pub dest: SocketAddrV4,
    pub peer_cid: u32,
    pub race_spin: bool,
    pub v_rcvlowat: i32,
    /// 0: install the flow vsock right after connect (the design); 1: only after Paired, no re-arm; 2: after Paired + SO_RCVLOWAT re-arm
    pub v_late: u32,
    pub a_rearm: bool,
    pub p2_kick: bool,
    pub intake_rcvbuf: i32,
    /// false: ignore the kernel's redirect-refused notice (to show what the owner sees without it)
    pub drop_notice: bool,
    pub chaos_v_reset_permille: u32,
    pub events: Option<String>,
    pub sample_every: u64,
    pub pin: Option<usize>,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Ctl {
    Paired(u32),
    HalfClose { id: u32, count: u64 },
}

impl Ctl {
    pub fn enc(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        match *self {
            Ctl::Paired(id) => {
                b[0..4].copy_from_slice(&1u32.to_ne_bytes());
                b[4..8].copy_from_slice(&id.to_ne_bytes());
            }
            Ctl::HalfClose { id, count } => {
                b[0..4].copy_from_slice(&2u32.to_ne_bytes());
                b[4..8].copy_from_slice(&id.to_ne_bytes());
                b[8..16].copy_from_slice(&count.to_ne_bytes());
            }
        }
        b
    }
    pub fn dec(b: &[u8]) -> Option<Ctl> {
        let k = u32::from_ne_bytes(b[0..4].try_into().ok()?);
        let id = u32::from_ne_bytes(b[4..8].try_into().ok()?);
        let count = u64::from_ne_bytes(b[8..16].try_into().ok()?);
        match k {
            1 => Some(Ctl::Paired(id)),
            2 => Some(Ctl::HalfClose { id, count }),
            _ => None,
        }
    }
}

pub struct Chan {
    pub q: Mutex<VecDeque<Ctl>>,
    pub efd: OwnedFd,
}
impl Chan {
    pub fn new() -> Arc<Chan> {
        Arc::new(Chan { q: Mutex::new(VecDeque::new()), efd: eventfd() })
    }
    pub fn push(&self, m: Ctl) {
        self.q.lock().unwrap().push_back(m);
        efd_signal(self.efd.as_raw_fd());
    }
}

pub enum Peer {
    Local(Arc<Chan>),
    Vsock(RawFd),
}
impl Peer {
    fn send(&self, m: Ctl) {
        match self {
            Peer::Local(c) => c.push(m),
            Peer::Vsock(fd) => {
                let b = m.enc();
                let mut off = 0;
                while off < b.len() {
                    let n = send_blocking(*fd, &b[off..]);
                    if n <= 0 {
                        return;
                    }
                    off += n as usize;
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Stats(pub BTreeMap<String, u64>);
impl Stats {
    pub fn inc(&mut self, k: &str) {
        *self.0.entry(k.to_string()).or_insert(0) += 1;
    }
    pub fn add(&mut self, k: &str, n: u64) {
        *self.0.entry(k.to_string()).or_insert(0) += n;
    }
    pub fn max(&mut self, k: &str, n: u64) {
        let e = self.0.entry(k.to_string()).or_insert(0);
        if n > *e {
            *e = n;
        }
    }
}

struct Flow {
    id: u32,
    o: OwnedFd,
    v: OwnedFd,
    o_ck: u64,
    v_ck: u64,
    cell: [u32; 2],
    gen: [u64; 2],
    park_slot: Option<u32>,
    paired: bool,
    eof: [bool; 2],
    eof_t: [u64; 2],
    goal: [Option<u64>; 2],
    armed: [bool; 2],
    done: [bool; 2],
    shut: [bool; 2],
    done_t: [u64; 2],
    how: [&'static str; 2],
    peer_count: Option<u64>,
    hup: [bool; 2],
    diag: [u64; 6],
    t_start: u64,
}

pub struct Role {
    cfg: RoleCfg,
    bpf: Arc<Bpf>,
    ep: Epoll,
    lst: OwnedFd,
    is_guest: bool,
    ring: RingBuf<MapData>,
    chan: Arc<Chan>,
    peer: Peer,
    ctl_fd: Option<RawFd>,
    ctl_buf: Vec<u8>,
    flows: BTreeMap<u32, Flow>,
    fdmap: HashMap<RawFd, (u32, u8)>,
    free_cells: Vec<u32>,
    park: Vec<Option<(OwnedFd, OwnedFd)>>,
    park_lst: Option<OwnedFd>,
    cell_owner: HashMap<u32, (u32, u8)>,
    pending_rec: HashMap<u32, Rec>,
    early_half: HashMap<u32, u64>,
    early_apply: Vec<(u32, Rec)>,
    pub live: Arc<std::sync::atomic::AtomicUsize>,
    pub stats: Stats,
    events: Option<File>,
    ev_n: u64,
    next_id: Arc<AtomicU32>,
    chaos: Vec<(u64, u32)>,
    rng: u64,
}

fn tcp_listener_v4(port: u16) -> OwnedFd {
    tcp_listen(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).expect("tcp listen")
}

impl Role {
    pub fn new(cfg: RoleCfg, bpf: Arc<Bpf>, chan: Arc<Chan>, peer: Peer, ctl_fd: Option<RawFd>, next_id: Arc<AtomicU32>) -> Role {
        let is_guest = cfg.name == "guest";
        let ep = Epoll::new();
        let lst = if is_guest {
            bpf.intake_set(cfg.intake_port as u32);
            if cfg.intake_rcvbuf > 0 {
                // children inherit the listener's SO_RCVBUF: a window that cannot close before accept()
                let l = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
                setopt_i32(l.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1).unwrap();
                setopt_i32(l.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, cfg.intake_rcvbuf).unwrap();
                bind4(l.as_raw_fd(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, cfg.intake_port)).unwrap();
                cvt(unsafe { libc::listen(l.as_raw_fd(), 4096) }).unwrap();
                l
            } else {
                tcp_listener_v4(cfg.intake_port)
            }
        } else {
            vsock_listen(libc::VMADDR_CID_ANY, VPORT, libc::SOCK_STREAM).expect("vsock listen")
        };
        set_nonblock(lst.as_raw_fd());
        ep.add_tok(lst.as_raw_fd(), libc::EPOLLIN as u32, TOK_LST);
        ep.add_tok(chan.efd.as_raw_fd(), libc::EPOLLIN as u32, TOK_CHAN);
        let ring = bpf.take_ring(if is_guest { "RB_G" } else { "RB_H" });
        ep.add_tok(ring.as_raw_fd(), libc::EPOLLIN as u32, TOK_RING);
        if let Some(fd) = ctl_fd {
            ep.add_tok(fd, libc::EPOLLIN as u32, TOK_CTL);
        }
        let base = if is_guest { 0 } else { ROLE_SPLIT };
        let mut free_cells = vec![];
        let mut park = vec![];
        let mut park_lst = None;
        if is_guest {
            let pl = tcp_listener_v4(0);
            park_lst = Some(pl);
            park = (0..cfg.nflows).map(|_| None).collect();
        } else {
            for k in (0..cfg.nflows).rev() {
                free_cells.push(base + 2 * k);
            }
        }
        let events = cfg.events.as_ref().map(|p| File::create(p).expect("events file"));
        let rng = cfg.seed ^ 0x9e37_79b9_7f4a_7c15;
        let mut r = Role {
            cfg,
            bpf,
            ep,
            lst,
            is_guest,
            ring,
            chan,
            peer,
            ctl_fd,
            ctl_buf: vec![],
            flows: BTreeMap::new(),
            fdmap: HashMap::new(),
            free_cells,
            park,
            park_lst,
            cell_owner: HashMap::new(),
            pending_rec: HashMap::new(),
            early_half: HashMap::new(),
            early_apply: vec![],
            live: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            stats: Stats::default(),
            events,
            ev_n: 0,
            next_id,
            chaos: vec![],
            rng,
        };
        if is_guest {
            for s in 0..r.cfg.nflows {
                r.refresh_park(s);
            }
        }
        r
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// (Re)create the loopback parking pair of a slot and hand the slot to the kernel.
    fn refresh_park(&mut self, slot: u32) {
        if let Some((p1, p2)) = self.park[slot as usize].take() {
            let c1 = cookie(p1.as_raw_fd());
            let c2 = cookie(p2.as_raw_fd());
            self.bpf.sock_remove("SOCKS", c1);
            self.bpf.sock_remove("VERD", c2);
            self.bpf.route_del(c2);
        }
        let lport = local4(self.park_lst.as_ref().unwrap().as_raw_fd()).unwrap().port();
        let p1 = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
        connect4(p1.as_raw_fd(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, lport)).expect("park connect");
        let p2 = accept(self.park_lst.as_ref().unwrap().as_raw_fd()).expect("park accept");
        let c1 = cookie(p1.as_raw_fd());
        self.bpf.sock_insert("SOCKS", c1, p1.as_raw_fd()).expect("P1 install");
        self.bpf.park_set(slot, c1);
        // the cell pair of a slot starts clean; its epoch only moves when a flow ends
        self.park[slot as usize] = Some((p1, p2));
        self.bpf.tcpfree_push(slot);
    }

    fn ev(&mut self, mut v: serde_json::Value, anomaly: bool) {
        self.ev_n += 1;
        let sample = self.cfg.sample_every.max(1);
        if self.events.is_none() {
            return;
        }
        if !anomaly && self.ev_n > 20 && self.ev_n % sample != 0 {
            return;
        }
        if let Some(m) = v.as_object_mut() {
            m.insert("role".into(), json!(self.cfg.name));
            m.insert("t_ns".into(), json!(mono_ns()));
        }
        if let Some(f) = self.events.as_mut() {
            let _ = writeln!(f, "{v}");
        }
    }

    pub fn run(&mut self, stop: Arc<AtomicBool>) {
        if let Some(c) = self.cfg.pin {
            pin_cpu(c);
        }
        while !stop.load(Ordering::Relaxed) {
            let mut timeout = 100;
            if !self.chaos.is_empty() {
                timeout = 5;
            }
            let evs = self.ep.wait_tok(timeout);
            // control inputs and completion records first, then listener, then flow sockets
            for &(tok, _) in evs.iter() {
                match tok {
                    TOK_RING => self.on_ring(),
                    TOK_CHAN => {
                        efd_drain(self.chan.efd.as_raw_fd());
                        self.on_chan();
                    }
                    TOK_CTL => self.on_ctl_fd(),
                    _ => {}
                }
            }
            for &(tok, _) in evs.iter() {
                if tok == TOK_LST {
                    self.on_accept();
                }
            }
            for &(tok, ev) in evs.iter() {
                if tok < (1 << 62) {
                    self.on_flow_tok(tok, ev);
                }
            }
            // the ring may carry records whose wakeup raced the epoll set; poll it every turn
            self.on_ring();
            self.run_chaos();
            self.live.store(self.flows.len(), Ordering::Relaxed);
        }
    }

    // ------------------------------------------------------------ accept / pairing
    fn on_accept(&mut self) {
        loop {
            let n = unsafe { libc::accept4(self.lst.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
            if n < 0 {
                return;
            }
            let fd = unsafe { OwnedFd::from_raw_fd(n) };
            if self.is_guest {
                self.guest_intake(fd);
            } else {
                self.host_vsock(fd);
            }
        }
    }

    fn claim_cells(&mut self, id: u32, cells: [u32; 2], flow_dirs: bool) -> [u64; 2] {
        let _ = flow_dirs;
        let mut gens = [0u64; 2];
        for d in 0..2 {
            let g = self.bpf.a_gen(cells[d]).load(Ordering::SeqCst);
            gens[d] = g;
            self.cell_owner.insert(cells[d], (id, d as u8));
            if let Some(rec) = self.pending_rec.remove(&cells[d]) {
                // a kernel-armed record can legitimately precede the owner's accept
                if rec.gen == g {
                    self.stats.inc("record_before_accept");
                    self.early_apply.push((cells[d], rec));
                } else {
                    self.stats.inc("VIOLATION_stale_record");
                }
            }
        }
        gens
    }

    fn guest_intake(&mut self, a: OwnedFd) {
        let a_ck = cookie(a.as_raw_fd());
        // bytes that arrived before accept() find sk->sk_socket NULL in sk_psock_verdict_data_ready and
        // stay queued: the owner re-arms the child right after accept() (payload-free SO_RCVLOWAT)
        let acc_br = tcp_bytes_received(a.as_raw_fd()).unwrap_or(0);
        let acc_inq = inq(a.as_raw_fd()) as u64;
        if self.cfg.a_rearm {
            let _ = set_rcvlowat(a.as_raw_fd(), 1);
        }
        let Some(slot) = self.bpf.park_of(a_ck) else {
            self.stats.inc("intake_without_park_slot");
            return;
        };
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let cells = [slot * 2, slot * 2 + 1];
        let gens = self.claim_cells(id, cells, true);
        let v = match vsock_connect(self.cfg.peer_cid, VPORT, libc::SOCK_STREAM) {
            Ok(v) => v,
            Err(e) => {
                self.stats.inc("vsock_connect_fail");
                self.ev(json!({"ev":"vsock_connect_fail","err":e.to_string()}), true);
                self.release_cells(cells);
                self.refresh_park(slot);
                return;
            }
        };
        let v_ck = cookie(v.as_raw_fd());
        if self.cfg.v_rcvlowat > 1 {
            let _ = set_rcvlowat(v.as_raw_fd(), self.cfg.v_rcvlowat);
        }
        self.bpf.dst_set(v_ck, cells[0]);
        self.bpf.dst_set(a_ck, cells[1]);
        if self.cfg.v_late == 0 {
            self.bpf.route_set(v_ck, a_ck, cells[1]);
            for m in ["SOCKS", "VERD"] {
                if let Err(e) = self.bpf.sock_insert(m, v_ck, v.as_raw_fd()) {
                    self.stats.inc("v_install_fail");
                    self.ev(json!({"ev":"v_install_fail","err":e}), true);
                }
            }
        }
        let mut hdr = [0u8; 16];
        hdr[0..4].copy_from_slice(&HDR_MAGIC.to_ne_bytes());
        hdr[4..8].copy_from_slice(&id.to_ne_bytes());
        send_blocking(v.as_raw_fd(), &hdr);
        let (ao, vo) = (a.as_raw_fd(), v.as_raw_fd());
        let f = Flow {
            id,
            o: a,
            v,
            o_ck: a_ck,
            v_ck,
            cell: cells,
            gen: gens,
            park_slot: Some(slot),
            paired: false,
            eof: [false; 2],
            eof_t: [0; 2],
            goal: [None; 2],
            armed: [false; 2],
            done: [false; 2],
            shut: [false; 2],
            done_t: [0; 2],
            how: ["", ""],
            peer_count: self.early_half.remove(&id),
            hup: [false; 2],
            diag: [0; 6],
            t_start: mono_ns(),
        };
        let mut f = f;
        f.diag[0] = acc_br;
        f.diag[1] = acc_inq;
        self.flows.insert(id, f);
        let fl = (libc::EPOLLRDHUP | libc::EPOLLERR | libc::EPOLLHUP | libc::EPOLLET) as u32;
        self.ep.add_tok(ao, fl, flow_tok(id, 0));
        self.ep.add_tok(vo, fl, flow_tok(id, 1));
        self.stats.inc("flows_started");
        self.apply_early();
    }

    fn host_vsock(&mut self, v: OwnedFd) {
        let mut hdr = [0u8; 16];
        set_timeouts(v.as_raw_fd(), Duration::from_secs(10));
        let n = recv_blocking(v.as_raw_fd(), &mut hdr);
        if n != 16 || u32::from_ne_bytes(hdr[0..4].try_into().unwrap()) != HDR_MAGIC {
            self.stats.inc("bad_flow_header");
            return;
        }
        let id = u32::from_ne_bytes(hdr[4..8].try_into().unwrap());
        let Some(base) = self.free_cells.pop() else {
            self.stats.inc("host_out_of_cells");
            return;
        };
        let cells = [base, base + 1];
        let gens = self.claim_cells(id, cells, true);
        let v_ck = cookie(v.as_raw_fd());
        if self.cfg.v_rcvlowat > 1 {
            let _ = set_rcvlowat(v.as_raw_fd(), self.cfg.v_rcvlowat);
        }
        let d = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
        let d_ck = cookie(d.as_raw_fd());
        // dir0: D -> V' (cell0, counted at V'); dir1: V' -> D (cell1, counted at D)
        self.bpf.dst_set(v_ck, cells[0]);
        self.bpf.dst_set(d_ck, cells[1]);
        self.bpf.route_set(d_ck, v_ck, cells[0]);
        self.bpf.route_set(v_ck, d_ck, cells[1]);
        self.bpf.arm_set(d_ck);
        for m in ["SOCKS", "VERD"] {
            if let Err(e) = self.bpf.sock_insert(m, v_ck, v.as_raw_fd()) {
                self.stats.inc("v_install_fail");
                self.ev(json!({"ev":"v_install_fail","err":e}), true);
            }
        }
        if let Err(e) = connect4_timeout(d.as_raw_fd(), self.cfg.dest, Duration::from_secs(10)) {
            self.stats.inc("dest_connect_fail");
            self.ev(json!({"ev":"dest_connect_fail","err":e.to_string()}), true);
            // roll back without a flow
            self.bpf.route_del(d_ck);
            self.bpf.route_del(v_ck);
            self.bpf.dst_del(v_ck);
            self.bpf.dst_del(d_ck);
            self.bpf.arm_del(d_ck);
            self.bpf.sock_remove("SOCKS", v_ck);
            self.bpf.sock_remove("VERD", v_ck);
            self.release_cells(cells);
            self.free_cells.push(base);
            return;
        }
        let (dd, vv) = (d.as_raw_fd(), v.as_raw_fd());
        let f = Flow {
            id,
            o: d,
            v,
            o_ck: d_ck,
            v_ck,
            cell: cells,
            gen: gens,
            park_slot: None,
            paired: true,
            eof: [false; 2],
            eof_t: [0; 2],
            goal: [None; 2],
            armed: [false; 2],
            done: [false; 2],
            shut: [false; 2],
            done_t: [0; 2],
            how: ["", ""],
            peer_count: self.early_half.remove(&id),
            hup: [false; 2],
            diag: [0; 6],
            t_start: mono_ns(),
        };
        self.flows.insert(id, f);
        let fl = (libc::EPOLLRDHUP | libc::EPOLLERR | libc::EPOLLHUP | libc::EPOLLET) as u32;
        self.ep.add_tok(dd, fl, flow_tok(id, 0));
        self.ep.add_tok(vv, fl, flow_tok(id, 1));
        self.stats.inc("flows_started");
        self.peer.send(Ctl::Paired(id));
        self.apply_early();
    }

    fn on_chan(&mut self) {
        loop {
            let m = self.chan.q.lock().unwrap().pop_front();
            match m {
                Some(m) => self.on_ctl(m),
                None => return,
            }
        }
    }

    fn on_ctl_fd(&mut self) {
        let fd = self.ctl_fd.unwrap();
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
            if n <= 0 {
                break;
            }
            self.ctl_buf.extend_from_slice(&buf[..n as usize]);
        }
        while self.ctl_buf.len() >= 24 {
            let frame: Vec<u8> = self.ctl_buf.drain(..24).collect();
            if let Some(m) = Ctl::dec(&frame) {
                self.on_ctl(m);
            }
        }
    }

    fn on_ctl(&mut self, m: Ctl) {
        match m {
            Ctl::Paired(id) => {
                if !self.is_guest {
                    return;
                }
                self.finish_pairing(id);
            }
            Ctl::HalfClose { id, count } => {
                if let Some(f) = self.flows.get_mut(&id) {
                    f.peer_count = Some(count);
                    if f.eof[1] && f.goal[1].is_none() {
                        self.vsock_source_ready(id);
                    }
                } else {
                    self.early_half.insert(id, count);
                }
            }
        }
    }

    /// Guest: the host owner installed the destination leg. Route the parking cell's far end to
    /// the flow vsock (the kernel then forwards what was parked), and re-arm it.
    fn finish_pairing(&mut self, id: u32) {
        let (slot, v_ck) = match self.flows.get(&id) {
            Some(f) => (f.park_slot.unwrap(), f.v_ck),
            None => return,
        };
        if self.cfg.v_late != 0 {
            let (a_ck, c1, vfd) = {
                let f = &self.flows[&id];
                (f.o_ck, f.cell[1], f.v.as_raw_fd())
            };
            self.bpf.route_set(v_ck, a_ck, c1);
            for m in ["SOCKS", "VERD"] {
                if let Err(e) = self.bpf.sock_insert(m, v_ck, vfd) {
                    self.stats.inc("v_install_fail");
                    self.ev(json!({"ev":"v_install_fail","err":e}), true);
                }
            }
            if self.cfg.v_late == 2 {
                let _ = set_rcvlowat(vfd, 1);
            }
        }
        let p2 = self.park[slot as usize].as_ref().unwrap().1.as_raw_fd();
        let p2_ck = cookie(p2);
        let c0 = self.flows[&id].cell[0];
        self.bpf.route_set_x(p2_ck, v_ck, c0, true);
        if let Err(e) = self.bpf.sock_insert("VERD", p2_ck, p2) {
            self.stats.inc("p2_install_fail");
            self.ev(json!({"ev":"p2_install_fail","err":e}), true);
        }
        let (p2_inq_before, a_br) = (inq(p2) as u64, self.flows.get(&id).map(|f| tcp_bytes_received(f.o.as_raw_fd()).unwrap_or(0)).unwrap_or(0));
        // re-arm: tcp_set_rcvlowat -> tcp_data_ready -> read_skb drains what was queued
        if self.cfg.p2_kick {
            let _ = set_rcvlowat(p2, 1);
        }
        if let Some(f) = self.flows.get_mut(&id) {
            f.paired = true;
            f.diag[2] = p2_inq_before;
            f.diag[3] = a_br;
            f.diag[4] = inq(p2) as u64;
        }
        for d in 0..2u8 {
            self.try_shutdown(id, d);
        }
        if self.cfg.chaos_v_reset_permille > 0 {
            let r = self.rand();
            if (r % 1000) < self.cfg.chaos_v_reset_permille as u64 {
                let when = mono_ns() + 1_000_000 + (self.rand() % 20_000_000);
                self.chaos.push((when, id));
            }
        }
    }

    fn run_chaos(&mut self) {
        if self.chaos.is_empty() {
            return;
        }
        let now = mono_ns();
        let due: Vec<u32> = self.chaos.iter().filter(|(t, _)| *t <= now).map(|(_, id)| *id).collect();
        self.chaos.retain(|(t, _)| *t > now);
        for id in due {
            if let Some(f) = self.flows.get(&id) {
                // reset the flow vsock: the peer's vsock leaves ESTABLISHED
                let l = libc::linger { l_onoff: 1, l_linger: 0 };
                unsafe { libc::setsockopt(f.v.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
                unsafe { libc::shutdown(f.v.as_raw_fd(), libc::SHUT_RDWR) };
                self.stats.inc("chaos_v_reset");
            }
        }
    }

    // ------------------------------------------------------------ socket events
    fn on_flow_tok(&mut self, tok: u64, ev: u32) {
        let (id, which) = ((tok >> 1) as u32, (tok & 1) as u8);
        let Some(fl) = self.flows.get(&id) else {
            self.stats.inc("stale_event_for_finished_flow");
            return;
        };
        let fd = if which == 0 { fl.o.as_raw_fd() } else { fl.v.as_raw_fd() };
        if ev & libc::EPOLLERR as u32 != 0 {
            let err = so_error(fd);
            self.abort(id, "epollerr", json!({"which": which, "bits": epoll_bits(ev), "so_error": err}));
            return;
        }
        // HUP on the TARGET leg of a direction that is draining (source EOF seen or armed) and not
        // complete: the target can never accept the rest. Leg `which` is the target of direction 1-which.
        let tdir = (1 - which) as usize;
        if ev & libc::EPOLLHUP as u32 != 0 {
            let f = self.flows.get_mut(&id).unwrap();
            f.hup[which as usize] = true;
            if (f.armed[tdir] || f.eof[tdir]) && !f.done[tdir] {
                self.abort(id, "target_hup_mid_drain", json!({"which": which, "bits": epoll_bits(ev)}));
                return;
            }
        }
        if ev & libc::EPOLLRDHUP as u32 != 0 && ev & libc::EPOLLHUP as u32 != 0 && which == 1 && self.cfg.arm != ArmMode::None {
            // the flow vsock ended in both directions and the peer never announced a half-close:
            // the peer owner's loss, never a half-close (re-check the control inbox first)
            self.on_chan();
            let f = &self.flows[&id];
            if f.peer_count.is_none() && !f.done[1] {
                self.abort(id, "unannounced_vsock_end", json!({"bits": epoll_bits(ev)}));
                return;
            }
        }
        if ev & libc::EPOLLRDHUP as u32 != 0 {
            self.on_eof(id, which);
        }
    }

    /// Source EOF of direction `d` (0: outer leg is the source; 1: flow vsock is the source).
    fn on_eof(&mut self, id: u32, d: u8) {
        let t = mono_ns();
        let (o_fd, du) = {
            let f = self.flows.get_mut(&id).unwrap();
            if f.eof[d as usize] {
                return;
            }
            f.eof[d as usize] = true;
            f.eof_t[d as usize] = t;
            (f.o.as_raw_fd(), d as usize)
        };
        {
            // direction d's target leg: d0 -> flow vsock (which=1), d1 -> outer leg (which=0)
            let tw = if d == 0 { 1 } else { 0 };
            if self.flows[&id].hup[tw] && self.cfg.arm != ArmMode::None {
                self.abort(id, "target_hup_before_drain", json!({"dir": d}));
                return;
            }
        }
        if d == 0 {
            let br = tcp_bytes_received(o_fd).unwrap_or(0);
            let goal = br.saturating_sub(1);
            let c = self.flows[&id].cell[0];
            let sent = self.bpf.sent_max(c);
            let fwd = self.bpf.a_fwd(c).load(Ordering::SeqCst);
            self.stats.inc("tcp_eof_events");
            if sent < goal {
                self.stats.inc("tcp_eof_sent_lt_goal");
                self.stats.add("tcp_eof_deficit_bytes", goal - sent);
                self.stats.max("tcp_eof_deficit_max", goal - sent);
            }
            if fwd < goal {
                self.stats.inc("tcp_eof_fwd_lt_goal");
            }
            self.ev(json!({"ev":"eof","flow":id,"dir":0,"goal":goal,"sent":sent,"fwd":fwd,"bytes_received":br}), false);
            let f = self.flows.get_mut(&id).unwrap();
            f.goal[0] = Some(goal);
            match self.cfg.arm {
                ArmMode::None => self.complete(id, 0, "naive"),
                ArmMode::Owner => self.arm_owner(id, 0, goal),
                ArmMode::Kernel => { /* the kernel armed it in STATE_CB; completion arrives as a record */ }
            }
        } else {
            let _ = du;
            let f = &self.flows[&id];
            if f.peer_count.is_some() {
                self.vsock_source_ready(id);
            } else if self.cfg.arm == ArmMode::None {
                self.complete(id, 1, "naive");
            } else {
                self.stats.inc("vsock_eof_before_halfclose_msg");
            }
        }
    }

    /// vsock-source direction: EOF seen and the peer's HalfClose count is known.
    fn vsock_source_ready(&mut self, id: u32) {
        if self.flows[&id].hup[0] && self.cfg.arm != ArmMode::None {
            self.abort(id, "target_hup_before_drain", json!({"dir": 1}));
            return;
        }
        let (count, c) = {
            let f = &self.flows[&id];
            (f.peer_count.unwrap(), f.cell[1])
        };
        let sent = self.bpf.sent_max(c);
        let fwd = self.bpf.a_fwd(c).load(Ordering::SeqCst);
        self.stats.inc("vsock_eof_events");
        if sent < count {
            self.stats.inc("vsock_eof_sent_lt_goal");
            self.stats.add("vsock_eof_deficit_bytes", count - sent);
        }
        if fwd < count {
            self.stats.inc("vsock_eof_fwd_lt_goal");
        }
        self.ev(json!({"ev":"eof","flow":id,"dir":1,"goal":count,"sent":sent,"fwd":fwd}), false);
        self.flows.get_mut(&id).unwrap().goal[1] = Some(count);
        match self.cfg.arm {
            ArmMode::None => self.complete(id, 1, "naive"),
            _ => self.arm_owner(id, 1, count),
        }
    }

    /// Set-goal-then-check. The kernel's fexit does fetch_add(sent) -> check armed -> CAS(fired);
    /// the owner does store(goal) -> store(armed) -> load(sent) -> CAS(fired). Whoever wins the
    /// CAS reports completion; the loser reports nothing.
    fn arm_owner(&mut self, id: u32, d: u8, goal: u64) {
        let c = self.flows[&id].cell[d as usize];
        if self.cfg.race_spin {
            // force the arm into the window of the final skb_send_sock calls
            let slack = self.rand() % 65536;
            let t0 = mono_ns();
            while self.bpf.sent_max(c) + slack < goal && mono_ns() - t0 < 1_000_000_000 {
                std::hint::spin_loop();
            }
        }
        self.bpf.a_goal(c).store(goal, Ordering::SeqCst);
        self.bpf.a_armed(c).store(1, Ordering::SeqCst);
        self.flows.get_mut(&id).unwrap().armed[d as usize] = true;
        let sent = self.bpf.sent_max(c);
        if sent >= goal {
            if self.bpf.a_fired(c).compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                self.stats.inc("complete_owner_cas");
                self.complete(id, d, "owner_cas");
            } else {
                self.stats.inc("owner_lost_cas");
            }
        } else {
            self.stats.inc("owner_waits_for_record");
        }
    }

    // ------------------------------------------------------------ ring records
    fn on_ring(&mut self) {
        loop {
            let rec = match self.ring.next() {
                Some(item) => Rec::parse(&item),
                None => return,
            };
            let Some(rec) = rec else {
                self.stats.inc("bad_record");
                continue;
            };
            self.stats.inc("records");
            match self.cell_owner.get(&rec.cell).copied() {
                Some((id, d)) => self.apply_rec(id, d, rec),
                None => {
                    let cur = self.bpf.a_gen(rec.cell).load(Ordering::SeqCst);
                    if rec.gen < cur && rec.kind == 3 {
                        // a redirect-refused notice for a direction whose flow already ended: benign, ignored by epoch
                        self.stats.inc("late_drop_notice_after_recycle");
                    } else if rec.gen < cur {
                        self.stats.inc("VIOLATION_completion_record_after_recycle");
                        self.ev(json!({"ev":"record_after_recycle","cell":rec.cell,"rec_gen":rec.gen,"cur_gen":cur,"sent":rec.sent,"goal":rec.goal}), true);
                    } else if self.pending_rec.insert(rec.cell, rec).is_some() {
                        self.stats.inc("VIOLATION_dup_pending_record");
                    }
                }
            }
        }
    }

    fn apply_rec(&mut self, id: u32, d: u8, rec: Rec) {
        let (gen, done, ogoal) = {
            let f = &self.flows[&id];
            (f.gen[d as usize], f.done[d as usize], f.goal[d as usize])
        };
        if rec.kind == 3 {
            // the kernel refused a redirect for this direction: bytes are gone, the flow cannot drain
            if !self.cfg.drop_notice {
                self.stats.inc("drop_notice_ignored");
                return;
            }
            let f_ok = self.flows.contains_key(&id);
            if f_ok && rec.gen == gen {
                self.stats.inc("record_redirect_dropped");
                self.abort(id, "redirect_dropped", json!({"dir": d, "first_dropped_len": rec.sent}));
            }
            return;
        }
        if rec.gen != gen {
            self.stats.inc("VIOLATION_record_gen_mismatch");
            self.ev(json!({"ev":"gen_mismatch","flow":id,"dir":d,"rec_gen":rec.gen,"gen":gen}), true);
            return;
        }
        if done {
            self.stats.inc("VIOLATION_duplicate_record");
            self.ev(json!({"ev":"dup_record","flow":id,"dir":d,"kind":rec.kind,"how":self.flows[&id].how[d as usize]}), true);
            return;
        }
        if rec.sent != rec.goal {
            self.stats.inc("record_sent_ne_goal");
        }
        if let Some(g) = ogoal {
            if g != rec.goal {
                self.stats.inc("VIOLATION_record_goal_ne_owner_goal");
            }
        }
        if rec.kind == 2 {
            self.stats.inc("complete_record_state_cb");
        } else {
            self.stats.inc("complete_record_fexit");
        }
        self.flows.get_mut(&id).unwrap().goal[d as usize] = Some(rec.goal);
        self.complete(id, d, if rec.kind == 2 { "record_state_cb" } else { "record_fexit" });
    }

    fn apply_early(&mut self) {
        let list: Vec<(u32, Rec)> = std::mem::take(&mut self.early_apply);
        for (cell, rec) in list {
            if let Some((id, d)) = self.cell_owner.get(&cell).copied() {
                self.apply_rec(id, d, rec);
            }
        }
    }

    // ------------------------------------------------------------ completion -> half-close
    fn complete(&mut self, id: u32, d: u8, how: &'static str) {
        let t = mono_ns();
        let (c, goal, eof_t) = {
            let f = self.flows.get_mut(&id).unwrap();
            if f.done[d as usize] {
                return;
            }
            f.done[d as usize] = true;
            f.how[d as usize] = how;
            f.done_t[d as usize] = t;
            (f.cell[d as usize], f.goal[d as usize], f.eof_t[d as usize])
        };
        let sent = self.bpf.sent_max(c);
        let fwd = self.bpf.a_fwd(c).load(Ordering::SeqCst);
        if d == 0 && how != "naive" {
            let o_fd = self.flows[&id].o.as_raw_fd();
            if let (Some(g), Some(br)) = (goal, tcp_bytes_received(o_fd)) {
                if br.saturating_sub(1) != g {
                    self.stats.inc("VIOLATION_goal_ne_tcpinfo_at_completion");
                }
            }
        }
        if let Some(g) = goal {
            if how != "naive" {
                let calls = self.bpf.a_sent_b(c).load(Ordering::SeqCst);
                if sent != g || fwd != g {
                    let f = &self.flows[&id];
                    let (slot, ock, vck) = (f.park_slot, f.o_ck, f.v_ck);
                    let (tcpi, sent_other) = (tcp_bytes_received(f.o.as_raw_fd()), self.bpf.sent_max(f.cell[1 - d as usize]));
                    self.ev(json!({"ev":"ANOMALY_count_mismatch","flow":id,"dir":d,"how":how,"goal":g,"sent":sent,"fwd":fwd,"sent_b":calls,"cell":c,"gen":self.flows[&id].gen[d as usize],
                        "slot":slot,"o_ck":ock,"v_ck":vck,"overlap_bytes_seen_by_verdict":self.bpf.overlap_of(ock),"diag_accept_br_inq_pair_p2inq_abr_p2inq_after":self.flows[&id].diag,"tcpi_bytes_received":tcpi,"sent_other_dir":sent_other,"eof_to_done_ns":t.saturating_sub(eof_t)}), true);
                }
                if sent != g {
                    self.stats.inc("VIOLATION_sent_ne_goal_at_completion");
                }
                if d == 0 && fwd != g {
                    self.stats.inc("tcp_fwd_ne_goal_at_completion");
                }
                if d == 1 && fwd != g {
                    self.stats.inc("vsock_fwd_ne_peer_count_at_completion");
                }
            }
        }
        if eof_t > 0 {
            self.stats.max("max_eof_to_complete_us", (t - eof_t) / 1000);
            self.stats.add("sum_eof_to_complete_us", (t - eof_t) / 1000);
            self.stats.inc("n_eof_to_complete");
        }
        self.ev(json!({"ev":"complete","flow":id,"dir":d,"how":how,"sent":sent,"goal":goal,"fwd":fwd,"eof_to_done_ns": if eof_t>0 {t-eof_t} else {0}}), false);
        self.try_shutdown(id, d);
    }

    fn try_shutdown(&mut self, id: u32, d: u8) {
        let (ok, fd, goal) = {
            let Some(f) = self.flows.get(&id) else { return };
            let du = d as usize;
            if !f.done[du] || f.shut[du] {
                return;
            }
            if d == 0 && !f.paired {
                return;
            }
            (true, if d == 0 { f.v.as_raw_fd() } else { f.o.as_raw_fd() }, f.goal[du].unwrap_or(0))
        };
        if !ok {
            return;
        }
        if d == 0 {
            // announce first (carries this direction's exact count), then shutdown the flow vsock
            self.peer.send(Ctl::HalfClose { id, count: goal });
        }
        if let Err(e) = shutdown_wr(fd) {
            self.stats.inc("shutdown_wr_err");
            self.ev(json!({"ev":"shutdown_wr_err","flow":id,"dir":d,"err":e.to_string()}), true);
        }
        let both = {
            let f = self.flows.get_mut(&id).unwrap();
            f.shut[d as usize] = true;
            f.shut[0] && f.shut[1]
        };
        if both {
            self.finish(id, false, "clean");
        }
    }

    // ------------------------------------------------------------ teardown
    fn abort(&mut self, id: u32, reason: &str, detail: serde_json::Value) {
        self.stats.inc(&format!("abort_{reason}"));
        let f = &self.flows[&id];
        let st = json!({"armed":f.armed,"done":f.done,"eof":f.eof,"goal":f.goal,
            "sent":[self.bpf.sent_max(f.cell[0]), self.bpf.sent_max(f.cell[1])]});
        self.ev(json!({"ev":"abort","flow":id,"reason":reason,"detail":detail,"state":st}), true);
        self.finish(id, true, reason);
    }

    fn finish(&mut self, id: u32, linger0: bool, why: &str) {
        let Some(f) = self.flows.remove(&id) else { return };
        self.stats.inc(if linger0 { "flows_aborted" } else { "flows_clean" });
        let _ = why;
        if !linger0 {
            // all sends are over: the two independent fexit counters must agree exactly
            for c in f.cell {
                let (sa, sb) = (self.bpf.a_sent(c).load(Ordering::SeqCst), self.bpf.a_sent_b(c).load(Ordering::SeqCst));
                if sa != sb {
                    self.stats.inc(if sa < sb { "counter_a_short_of_b" } else { "counter_b_short_of_a" });
                    self.ev(json!({"ev":"COUNTERS_DISAGREE","flow":f.id,"cell":c,"sent_a":sa,"sent_b":sb}), true);
                }
            }
        }
        let ovl = self.bpf.overlap_of(f.o_ck);
        if ovl > 0 {
            self.stats.inc("flows_with_tcp_rx_overlap");
            self.stats.add("tcp_rx_overlap_bytes", ovl);
        }
        self.bpf.seq_forget(f.o_ck);
        self.bpf.route_del(f.o_ck);
        self.bpf.route_del(f.v_ck);
        self.bpf.dst_del(f.o_ck);
        self.bpf.dst_del(f.v_ck);
        self.bpf.arm_del(f.o_ck);
        for m in ["SOCKS", "VERD"] {
            self.bpf.sock_remove(m, f.o_ck);
            self.bpf.sock_remove(m, f.v_ck);
        }
        self.ep.del(f.o.as_raw_fd());
        self.ep.del(f.v.as_raw_fd());
        if linger0 {
            let l = libc::linger { l_onoff: 1, l_linger: 0 };
            for fd in [f.o.as_raw_fd(), f.v.as_raw_fd()] {
                unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
            }
        }
        let cells = f.cell;
        let slot = f.park_slot;
        drop(f);
        self.release_cells(cells);
        match slot {
            Some(s) => self.refresh_park(s),
            None => self.free_cells.push(cells[0]),
        }
    }

    fn release_cells(&mut self, cells: [u32; 2]) {
        for c in cells {
            self.cell_owner.remove(&c);
            self.bpf.recycle_cell(c);
        }
    }

    /// Flows still alive (for the stuck-direction count).
    pub fn snapshot(&self) -> Vec<serde_json::Value> {
        self.flows
            .values()
            .map(|f| {
                let pb = |fd: RawFd| {
                    let mut p = libc::pollfd { fd, events: libc::POLLIN | libc::POLLOUT | libc::POLLRDHUP, revents: 0 };
                    unsafe { libc::poll(&mut p, 1, 0) };
                    epoll_bits(p.revents as u32)
                };
                json!({"flow":f.id,"age_ms":(mono_ns()-f.t_start)/1_000_000,"paired":f.paired,"eof":f.eof,"armed":f.armed,"done":f.done,"shut":f.shut,
                   "poll_o":pb(f.o.as_raw_fd()),"poll_v":pb(f.v.as_raw_fd()),"so_error_o":so_error(f.o.as_raw_fd()),"so_error_v":so_error(f.v.as_raw_fd()),
                   "fwd":[self.bpf.a_fwd(f.cell[0]).load(Ordering::SeqCst), self.bpf.a_fwd(f.cell[1]).load(Ordering::SeqCst)],"hup":f.hup,"cells":f.cell,"gen":f.gen,
                   "goal":f.goal,"sent":[self.bpf.sent_max(f.cell[0]), self.bpf.sent_max(f.cell[1])],
                   "peer_count":f.peer_count})
            })
            .collect()
    }

    pub fn cell_accounting(&self) -> serde_json::Value {
        json!({"free_cells": self.free_cells.len(), "cell_owner": self.cell_owner.len(), "capacity_flows": self.cfg.nflows, "pending_rec": self.pending_rec.len(), "early_half": self.early_half.len(), "chaos_pending": self.chaos.len()})
    }

    pub fn live_flows(&self) -> usize {
        self.flows.len()
    }
}
