//! Loader + typed handle over the drain ELF. Control path only; no payload I/O.
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use aya::{
    maps::{Array, HashMap, Map, PerCpuArray, Queue, RingBuf, SockHash},
    programs::{CgroupAttachMode, FExit, SkSkb, SockOps, TracePoint},
    Btf, Ebpf, EbpfLoader,
};

pub const NONE: u32 = u32::MAX;
pub const CELLS: usize = 4096;
pub const ROLE_SPLIT: u32 = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cell {
    pub sent: u64,
    pub goal: u64,
    pub armed: u64,
    pub fired: u64,
    pub fwd: u64,
    pub gen: u64,
    pub sent_b: u64,
    pub drops: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Route {
    pub target: u64,
    pub cell: u32,
    pub nofwd: u32,
}
unsafe impl aya::Pod for Route {}

#[derive(Clone, Copy, Debug)]
pub struct Rec {
    pub cell: u32,
    pub kind: u32,
    pub gen: u64,
    pub sent: u64,
    pub goal: u64,
    pub t_ns: u64,
}

impl Rec {
    pub fn parse(b: &[u8]) -> Option<Rec> {
        if b.len() < 40 {
            return None;
        }
        let u32_at = |o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_ne_bytes(b[o..o + 8].try_into().unwrap());
        Some(Rec { cell: u32_at(0), kind: u32_at(4), gen: u64_at(8), sent: u64_at(16), goal: u64_at(24), t_ns: u64_at(32) })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ShutRec {
    pub t_ns: u64,
    pub src_cid: u32,
    pub src_port: u32,
    pub dst_cid: u32,
    pub dst_port: u32,
    pub op: u32,
    pub flags: u32,
}

impl ShutRec {
    pub fn parse(b: &[u8]) -> Option<ShutRec> {
        if b.len() < 32 {
            return None;
        }
        let u32_at = |o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
        Some(ShutRec {
            t_ns: u64::from_ne_bytes(b[0..8].try_into().unwrap()),
            src_cid: u32_at(8),
            src_port: u32_at(12),
            dst_cid: u32_at(16),
            dst_port: u32_at(20),
            op: u32_at(24),
            flags: u32_at(28),
        })
    }
}

#[derive(Clone, Copy, Default)]
pub struct Attach {
    pub forwarder: bool, // verdict + fexit + sock_ops
    pub tracepoint: bool,
}

pub struct Bpf {
    pub e: Mutex<Ebpf>,
    pub cells: *mut Cell,
    pub fexit_fd: std::sync::atomic::AtomicI32,
    pub fexit_fd_b: std::sync::atomic::AtomicI32,
}
unsafe impl Send for Bpf {}
unsafe impl Sync for Bpf {}

impl Bpf {
    pub fn load(obj: &str, att: Attach, cgroup: Option<&str>) -> Result<Arc<Bpf>, String> {
        let mut e = EbpfLoader::new().load_file(obj).map_err(|x| format!("load ELF: {x:?}"))?;
        let mut fexit_fd: RawFd = -1;
        let mut fexit_fd_b: RawFd = -1;
        if att.forwarder {
            let vfd = SockHash::<_, u64>::try_from(e.map("VERD").unwrap()).unwrap().fd().try_clone().unwrap();
            let p: &mut SkSkb = e.program_mut("verdict").unwrap().try_into().unwrap();
            p.load().map_err(|x| format!("verdict load: {x:?}"))?;
            p.attach(&vfd).map_err(|x| format!("verdict attach: {x:?}"))?;
            let btf = Btf::from_sys_fs().map_err(|x| format!("btf: {x:?}"))?;
            let p: &mut FExit = e.program_mut("sent").unwrap().try_into().unwrap();
            p.load("skb_send_sock", &btf).map_err(|x| format!("fexit load: {x:?}"))?;
            p.attach().map_err(|x| format!("fexit attach: {x:?}"))?;
            fexit_fd = p.fd().map(|f| f.as_fd().as_raw_fd()).unwrap_or(-1);
            let p: &mut FExit = e.program_mut("sent_b").unwrap().try_into().unwrap();
            p.load("skb_send_sock", &btf).map_err(|x| format!("fexit_b load: {x:?}"))?;
            p.attach().map_err(|x| format!("fexit_b attach: {x:?}"))?;
            fexit_fd_b = p.fd().map(|f| f.as_fd().as_raw_fd()).unwrap_or(-1);
            let cg = File::open(cgroup.expect("cgroup path")).map_err(|x| format!("cgroup open: {x}"))?;
            let p: &mut SockOps = e.program_mut("flow_sockops").unwrap().try_into().unwrap();
            p.load().map_err(|x| format!("sockops load: {x:?}"))?;
            p.attach(&cg, CgroupAttachMode::Single).map_err(|x| format!("sockops attach: {x:?}"))?;
        }
        if att.tracepoint {
            let p: &mut TracePoint = e.program_mut("vsock_recv").unwrap().try_into().unwrap();
            p.load().map_err(|x| format!("tracepoint load: {x:?}"))?;
            p.attach("vsock", "virtio_transport_recv_pkt").map_err(|x| format!("tracepoint attach: {x:?}"))?;
        }
        let fd: RawFd = match e.map("CELLS").unwrap() {
            Map::Array(d) => d.fd().as_fd().as_raw_fd(),
            _ => panic!("CELLS not an array"),
        };
        let p = crate::sys::mmap_shared(fd, CELLS * std::mem::size_of::<Cell>()).map_err(|x| format!("mmap CELLS: {x}"))?;
        Ok(Arc::new(Bpf { e: Mutex::new(e), cells: p as *mut Cell, fexit_fd: std::sync::atomic::AtomicI32::new(fexit_fd), fexit_fd_b: std::sync::atomic::AtomicI32::new(fexit_fd_b) }))
    }

    // ---- cells (shared page; atomics on both sides)
    fn cp(&self, i: u32) -> *mut Cell {
        assert!((i as usize) < CELLS);
        unsafe { self.cells.add(i as usize) }
    }
    pub fn a_sent(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).sent) }
    }
    pub fn a_goal(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).goal) }
    }
    pub fn a_armed(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).armed) }
    }
    pub fn a_fired(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).fired) }
    }
    pub fn a_fwd(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).fwd) }
    }
    pub fn a_gen(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).gen) }
    }
    pub fn a_drops(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).drops) }
    }
    pub fn a_sent_b(&self, i: u32) -> &AtomicU64 {
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).sent_b) }
    }
    /// owner's view of bytes sent at the final hop: the larger of the two independent counters
    pub fn sent_max(&self, i: u32) -> u64 {
        self.a_sent(i).load(Ordering::SeqCst).max(self.a_sent_b(i).load(Ordering::SeqCst))
    }
    /// Reset a cell for the next flow: epoch + 1, everything else zero. Only after the cell's
    /// sockets are closed and its map entries removed.
    pub fn recycle_cell(&self, i: u32) -> u64 {
        let g = self.a_gen(i).load(Ordering::SeqCst) + 1;
        self.a_fired(i).store(0, Ordering::SeqCst);
        self.a_armed(i).store(0, Ordering::SeqCst);
        self.a_goal(i).store(0, Ordering::SeqCst);
        self.a_sent(i).store(0, Ordering::SeqCst);
        self.a_fwd(i).store(0, Ordering::SeqCst);
        self.a_sent_b(i).store(0, Ordering::SeqCst);
        unsafe { AtomicU64::from_ptr(&mut (*self.cp(i)).drops) }.store(0, Ordering::SeqCst);
        self.a_gen(i).store(g, Ordering::SeqCst);
        g
    }

    // ---- maps
    pub fn sock_insert(&self, map: &str, cookie: u64, fd: RawFd) -> Result<(), String> {
        let mut e = self.e.lock().unwrap();
        let mut m = SockHash::<_, u64>::try_from(e.map_mut(map).unwrap()).unwrap();
        m.insert(cookie, fd, 0).map_err(|x| format!("{map} insert: {x:?}"))
    }
    pub fn sock_remove(&self, map: &str, cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m = SockHash::<_, u64>::try_from(e.map_mut(map).unwrap()).unwrap();
        let _ = m.remove(&cookie);
    }
    pub fn route_set(&self, src: u64, target: u64, cell: u32) {
        self.route_set_x(src, target, cell, false)
    }
    pub fn route_set_x(&self, src: u64, target: u64, cell: u32, nofwd: bool) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, Route> = HashMap::try_from(e.map_mut("ROUTE").unwrap()).unwrap();
        m.insert(src, Route { target, cell, nofwd: nofwd as u32 }, 0).unwrap();
    }
    pub fn route_del(&self, src: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, Route> = HashMap::try_from(e.map_mut("ROUTE").unwrap()).unwrap();
        let _ = m.remove(&src);
    }
    pub fn dst_set(&self, cookie: u64, cell: u32) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("DSTCELL").unwrap()).unwrap();
        m.insert(cookie, cell, 0).unwrap();
    }
    pub fn dst_del(&self, cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("DSTCELL").unwrap()).unwrap();
        let _ = m.remove(&cookie);
    }
    pub fn arm_set(&self, cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("ARM").unwrap()).unwrap();
        m.insert(cookie, 1, 0).unwrap();
    }
    pub fn arm_del(&self, cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("ARM").unwrap()).unwrap();
        let _ = m.remove(&cookie);
    }
    pub fn intake_set(&self, port: u32) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u32, u32> = HashMap::try_from(e.map_mut("INTAKE").unwrap()).unwrap();
        m.insert(port, 1, 0).unwrap();
    }
    pub fn park_set(&self, slot: u32, p1_cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: Array<_, u64> = Array::try_from(e.map_mut("TCPPARK").unwrap()).unwrap();
        m.set(slot, p1_cookie, 0).unwrap();
    }
    pub fn tcpfree_push(&self, slot: u32) {
        let mut e = self.e.lock().unwrap();
        let mut q: Queue<_, u32> = Queue::try_from(e.map_mut("TCPFREE").unwrap()).unwrap();
        q.push(slot, 0).unwrap();
    }
    pub fn park_of(&self, cookie: u64) -> Option<u32> {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("PARK_OF").unwrap()).unwrap();
        let r = m.get(&cookie, 0).ok();
        if r.is_some() {
            let _ = m.remove(&cookie);
        }
        r
    }
    pub fn cfg_set(&self, i: u32, v: u32) {
        let mut e = self.e.lock().unwrap();
        let mut m: Array<_, u32> = Array::try_from(e.map_mut("CFG").unwrap()).unwrap();
        m.set(i, v, 0).unwrap();
    }
    pub fn counters(&self) -> Vec<u64> {
        let e = self.e.lock().unwrap();
        let a: PerCpuArray<_, u64> = PerCpuArray::try_from(e.map("CNT").unwrap()).unwrap();
        (0..32u32).map(|i| a.get(&i, 0).map(|v| v.iter().sum()).unwrap_or(0)).collect()
    }
    pub fn send_err_hist(&self) -> std::collections::BTreeMap<String, u64> {
        let e = self.e.lock().unwrap();
        let m: HashMap<_, u32, u64> = HashMap::try_from(e.map("ERRS").unwrap()).unwrap();
        m.iter().flatten().map(|(k, v)| (format!("errno{}{}", k & 0xffff, if k >> 16 != 0 { "/tracked_target" } else { "/untracked" }), v)).collect()
    }
    pub fn overlap_of(&self, cookie: u64) -> u64 {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u64> = HashMap::try_from(e.map_mut("OVL").unwrap()).unwrap();
        let r = m.get(&cookie, 0).unwrap_or(0);
        r
    }
    pub fn seq_forget(&self, cookie: u64) {
        let mut e = self.e.lock().unwrap();
        let mut m: HashMap<_, u64, u64> = HashMap::try_from(e.map_mut("OVL").unwrap()).unwrap();
        let _ = m.remove(&cookie);
        let mut m2: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("SEQNEXT").unwrap()).unwrap();
        let _ = m2.remove(&cookie);
    }
    /// recursion_misses of the fexit(skb_send_sock) program (calls the kernel skipped because the
    /// program was already running on this CPU), from the program fd's fdinfo.
    pub fn fexit_misses(&self) -> Option<u64> {
        let fd = self.fexit_fd.load(Ordering::Relaxed);
        if fd < 0 {
            return None;
        }
        let t = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok()?;
        t.lines().find_map(|l| l.strip_prefix("recursion_misses:").and_then(|v| v.trim().parse().ok()))
    }
    pub fn fexit_misses_b(&self) -> Option<u64> {
        let fd = self.fexit_fd_b.load(Ordering::Relaxed);
        if fd < 0 {
            return None;
        }
        let t = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok()?;
        t.lines().find_map(|l| l.strip_prefix("recursion_misses:").and_then(|v| v.trim().parse().ok()))
    }
    pub fn fexit_run_cnt(&self) -> Option<u64> {
        let fd = self.fexit_fd.load(Ordering::Relaxed);
        if fd < 0 {
            return None;
        }
        let t = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok()?;
        t.lines().find_map(|l| l.strip_prefix("run_cnt:").and_then(|v| v.trim().parse().ok()))
    }
    pub fn take_ring(&self, name: &str) -> RingBuf<aya::maps::MapData> {
        let mut e = self.e.lock().unwrap();
        RingBuf::try_from(e.take_map(name).unwrap()).unwrap()
    }
}

pub const CNT_NAMES: [&str; 29] = [
    "verdict_zero_len_drop",
    "verdict_noroute",
    "verdict_redirect_fail",
    "verdict_redirect_ok",
    "fexit_send_calls",
    "fire_fexit",
    "fire_state_cb",
    "ringbuf_output_fail",
    "parked",
    "park_empty",
    "install_fail",
    "state_cb_calls",
    "state_cb_nocell",
    "tracepoint_records",
    "tracepoint_drops",
    "fexit_send_err",
    "fexit_send_zero",
    "fexit_send_err_last_errno",
    "p2_verdicts",
    "fexit_send_untracked_target",
    "tcp_overlap_skbs",
    "tcp_overlap_bytes",
    "state_cb_est_to_closewait",
    "state_cb_armed",
    "state_cb_br_zero",
    "cb_flags_set_ok",
    "cb_flags_set_err",
    "state_cb_fw1_to_closing",
    "state_cb_fw2_to_close",
];
