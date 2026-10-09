//! V-24 guest side: a minimal model of the guest-flow owner (accept V_g on the
//! flow port, read the 16-byte request, connect C to the in-guest application,
//! hold, then observe V_g / tear down or route), plus the in-guest application,
//! the control-session client, a command channel and an observer (vsock_diag).
//! All observations are reported to the host as JSON lines over a vsock stream.
#![allow(clippy::all)]
use std::collections::BTreeMap;
use std::io;
use std::mem;
use std::os::fd::RawFd;
use std::sync::atomic::Ordering::SeqCst as SS;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const FLOW_PORT: u32 = 52421;
const CMD_PORT: u32 = 52420;
const HOST_CTL_PORT: u32 = 52423;
const HOST_RPT_PORT: u32 = 52424;
const APP_PORT: u16 = 8080;

static START: OnceLock<Instant> = OnceLock::new();
static HOLD: AtomicBool = AtomicBool::new(false); // acceptor paused
static CFG_HOLD_MS: AtomicU64 = AtomicU64::new(0);
static CFG_PAIRED_BLIND: AtomicBool = AtomicBool::new(false);
static CFG_CTL_IGNORE: AtomicBool = AtomicBool::new(false);
static CTL_DISCARDED: AtomicU64 = AtomicU64::new(0);
static CTL_ABORT_KNOWN: AtomicU64 = AtomicU64::new(0);
static CTL_UNEXPECTED: AtomicU64 = AtomicU64::new(0);
static SESSION_UPS: AtomicU64 = AtomicU64::new(0);
static LABEL: Mutex<String> = Mutex::new(String::new());
static SESSION: Mutex<Option<RawFd>> = Mutex::new(None);
static RPT: Mutex<Option<RawFd>> = Mutex::new(None);

#[derive(Default, Clone)]
struct FlowSt {
    aborted_ctl: bool,
    aborted_t: u64,
}
static FLOWS: Mutex<BTreeMap<u32, FlowSt>> = Mutex::new(BTreeMap::new());

fn now_us() -> u64 {
    START.get().unwrap().elapsed().as_micros() as u64
}
fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn write_all(fd: RawFd, mut b: &[u8]) -> io::Result<()> {
    while !b.is_empty() {
        let n = unsafe { libc::send(fd, b.as_ptr() as *const _, b.len(), libc::MSG_NOSIGNAL) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        b = &b[n as usize..];
    }
    Ok(())
}

fn rpt(line: &str) {
    let g = RPT.lock().unwrap();
    let mut s = String::with_capacity(line.len() + 1);
    s.push_str(line);
    s.push('\n');
    match *g {
        Some(fd) => {
            if write_all(fd, s.as_bytes()).is_err() {
                print!("RPT_FAIL {}", s);
            }
        }
        None => print!("RPT_NOCHAN {}", s),
    }
}

fn sockaddr_vm(cid: u32, port: u32) -> libc::sockaddr_vm {
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    a.svm_family = libc::AF_VSOCK as _;
    a.svm_cid = cid;
    a.svm_port = port;
    a
}

fn vsock_socket() -> io::Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd) }
}

fn vsock_listen(port: u32, backlog: i32) -> io::Result<RawFd> {
    let fd = vsock_socket()?;
    let a = sockaddr_vm(libc::VMADDR_CID_ANY, port);
    let rc = unsafe { libc::bind(fd, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::listen(fd, backlog) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn vsock_connect(cid: u32, port: u32) -> io::Result<RawFd> {
    let fd = vsock_socket()?;
    let a = sockaddr_vm(cid, port);
    let rc = unsafe { libc::connect(fd, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    Ok(fd)
}

fn accept(fd: RawFd) -> io::Result<RawFd> {
    loop {
        let r = unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        if r >= 0 {
            return Ok(r);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// accept() that also returns the peer port from the accept-time address (valid even if the
/// connection was already reset - getpeername() would fail with ENOTCONN in that case).
fn accept_inet(fd: RawFd) -> io::Result<(RawFd, u16)> {
    loop {
        let mut pa: libc::sockaddr_in = unsafe { mem::zeroed() };
        let mut pl = mem::size_of_val(&pa) as u32;
        let r = unsafe { libc::accept4(fd, &mut pa as *mut _ as *mut libc::sockaddr, &mut pl, libc::SOCK_CLOEXEC) };
        if r >= 0 {
            return Ok((r, u16::from_be(pa.sin_port)));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn set_rcvtimeo(fd: RawFd, ms: u64) {
    let tv = libc::timeval { tv_sec: (ms / 1000) as _, tv_usec: ((ms % 1000) * 1000) as _ };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const _, mem::size_of_val(&tv) as u32) };
}

fn set_linger0(fd: RawFd) {
    let l = libc::linger { l_onoff: 1, l_linger: 0 };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, mem::size_of_val(&l) as u32) };
}

fn so_error(fd: RawFd) -> i32 {
    let mut v: i32 = -1;
    let mut l = 4u32;
    unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut v as *mut _ as *mut _, &mut l) };
    v
}

fn poll1(fd: RawFd, events: i16, ms: i32) -> i16 {
    let mut p = libc::pollfd { fd, events, revents: 0 };
    unsafe { libc::poll(&mut p, 1, ms) };
    p.revents
}

fn peek(fd: RawFd) -> (isize, i32) {
    let mut b = [0u8; 1];
    let n = unsafe { libc::recv(fd, b.as_mut_ptr() as *mut _, 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
    (n, if n < 0 { errno() } else { 0 })
}

/// Read up to buf.len() bytes, looping until full / EOF / error. Returns (n, errno, eof).
fn read_full(fd: RawFd, buf: &mut [u8]) -> (usize, i32, bool) {
    let mut got = 0;
    while got < buf.len() {
        let n = unsafe { libc::recv(fd, buf[got..].as_mut_ptr() as *mut _, buf.len() - got, 0) };
        if n > 0 {
            got += n as usize;
        } else if n == 0 {
            return (got, 0, true);
        } else {
            let e = errno();
            if e == libc::EINTR {
                continue;
            }
            return (got, e, false);
        }
    }
    (got, 0, false)
}

fn lo_up() {
    unsafe {
        let s = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        let mut ifr: libc::ifreq = mem::zeroed();
        for (i, c) in b"lo".iter().enumerate() {
            ifr.ifr_name[i] = *c as _;
        }
        libc::ioctl(s, libc::SIOCGIFFLAGS as _, &mut ifr);
        ifr.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_RUNNING) as i16;
        let rc = libc::ioctl(s, libc::SIOCSIFFLAGS as _, &mut ifr);
        if rc < 0 {
            println!("GUEST_LO_UP_FAIL errno={}", errno());
        }
        libc::close(s);
    }
}

fn local_cid() -> i64 {
    unsafe {
        let fd = libc::open(b"/dev/vsock\0".as_ptr() as *const _, libc::O_RDONLY);
        if fd < 0 {
            return -(errno() as i64);
        }
        let mut cid: u32 = 0;
        let rc = libc::ioctl(fd, 0x7b9 as _, &mut cid as *mut u32);
        libc::close(fd);
        if rc < 0 { -(errno() as i64) } else { cid as i64 }
    }
}

// ---------------------------------------------------------------- vsock_diag
#[derive(Clone, Debug)]
struct DiagSock {
    state: u8,
    src_cid: u32,
    src_port: u32,
    dst_cid: u32,
    dst_port: u32,
}

fn vsock_diag() -> io::Result<Vec<DiagSock>> {
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 4 /*NETLINK_SOCK_DIAG*/);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut req: Vec<u8> = Vec::with_capacity(40);
        req.extend_from_slice(&40u32.to_ne_bytes());
        req.extend_from_slice(&20u16.to_ne_bytes()); // SOCK_DIAG_BY_FAMILY
        req.extend_from_slice(&0x301u16.to_ne_bytes()); // REQUEST|DUMP
        req.extend_from_slice(&1u32.to_ne_bytes());
        req.extend_from_slice(&0u32.to_ne_bytes());
        req.push(libc::AF_VSOCK as u8);
        req.push(0);
        req.extend_from_slice(&0u16.to_ne_bytes());
        req.extend_from_slice(&0xffff_ffffu32.to_ne_bytes()); // states
        req.extend_from_slice(&0u32.to_ne_bytes()); // ino
        req.extend_from_slice(&0u32.to_ne_bytes()); // show
        req.extend_from_slice(&[0u8; 8]); // cookie
        let rc = libc::send(fd, req.as_ptr() as *const _, req.len(), 0);
        if rc < 0 {
            let e = io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; 65536];
        'outer: loop {
            let n = libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0);
            if n <= 0 {
                break;
            }
            let mut off = 0usize;
            let n = n as usize;
            while off + 16 <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                let ty = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                if len < 16 || off + len > n {
                    break;
                }
                if ty == 3 {
                    break 'outer; // NLMSG_DONE
                }
                if ty == 2 {
                    libc::close(fd);
                    return Err(io::Error::new(io::ErrorKind::Other, "nlmsg error"));
                }
                if ty == 20 && len >= 16 + 28 {
                    let p = &buf[off + 16..off + len];
                    out.push(DiagSock {
                        state: p[2],
                        src_cid: u32::from_ne_bytes(p[4..8].try_into().unwrap()),
                        src_port: u32::from_ne_bytes(p[8..12].try_into().unwrap()),
                        dst_cid: u32::from_ne_bytes(p[12..16].try_into().unwrap()),
                        dst_port: u32::from_ne_bytes(p[16..20].try_into().unwrap()),
                    });
                }
                off += (len + 3) & !3;
            }
        }
        libc::close(fd);
        Ok(out)
    }
}

fn tcp_app_conns() -> Vec<String> {
    // every non-LISTEN TCP socket whose local or remote port is APP_PORT
    let mut v = Vec::new();
    let hex = format!("{:04X}", APP_PORT);
    if let Ok(t) = std::fs::read_to_string("/proc/net/tcp") {
        for l in t.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 4 {
                continue;
            }
            let lp = f[1].rsplit(':').next().unwrap_or("");
            let rp = f[2].rsplit(':').next().unwrap_or("");
            if (lp == hex || rp == hex) && f[3] != "0A" {
                v.push(format!("{}>{}:st{}", f[1], f[2], f[3]));
            }
        }
    }
    v
}

fn diag_json() -> String {
    match vsock_diag() {
        Ok(socks) => {
            let flow: Vec<&DiagSock> = socks.iter().filter(|s| s.src_port == FLOW_PORT && s.state != 10).collect();
            let mut by_state: BTreeMap<u8, usize> = BTreeMap::new();
            for s in &flow {
                *by_state.entry(s.state).or_default() += 1;
            }
            let st: Vec<String> = by_state.iter().map(|(k, v)| format!("\"{}\":{}", k, v)).collect();
            let all: Vec<String> = socks
                .iter()
                .filter(|s| socks.len() < 64 || s.src_port != FLOW_PORT)
                .map(|s| format!("\"{}:{}:{}>{}:{}\"", s.state, s.src_cid, s.src_port, s.dst_cid, s.dst_port))
                .collect();
            let tcp = tcp_app_conns();
            format!(
                "{{\"k\":\"diag\",\"flow_vsock_total\":{},\"flow_vsock_by_state\":{{{}}},\"tcp_app_conns\":{},\"tcp_app_list\":[{}],\"all_vsock_total\":{},\"all_vsock\":[{}]}}",
                flow.len(),
                st.join(","),
                tcp.len(),
                tcp.iter().take(20).map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(","),
                socks.len(),
                all.join(",")
            )
        }
        Err(e) => format!("{{\"k\":\"diag\",\"error\":\"{}\"}}", e),
    }
}

// ---------------------------------------------------------------- control session
fn send_ctl(ty: u8, flow: u32) -> bool {
    let fd = match *SESSION.lock().unwrap() {
        Some(fd) => fd,
        None => return false,
    };
    let mut m = [0u8; 8];
    m[0] = ty;
    m[4..8].copy_from_slice(&flow.to_le_bytes());
    write_all(fd, &m).is_ok()
}

fn control_loop() {
    loop {
        match vsock_connect(2, HOST_CTL_PORT) {
            Ok(fd) => {
                *SESSION.lock().unwrap() = Some(fd);
                let ups = SESSION_UPS.fetch_add(1, SS) + 1;
                let mut hello = [0u8; 8];
                hello[0] = 1;
                hello[4..8].copy_from_slice(&(ups as u32).to_le_bytes());
                let _ = write_all(fd, &hello);
                rpt(&format!("{{\"k\":\"session_up\",\"n\":{},\"t\":{}}}", ups, now_us()));
                loop {
                    let mut m = [0u8; 8];
                    let (n, e, eof) = read_full(fd, &mut m);
                    if n < 8 {
                        rpt(&format!(
                            "{{\"k\":\"session_lost\",\"n\":{},\"t\":{},\"partial\":{},\"errno\":{},\"eof\":{}}}",
                            ups,
                            now_us(),
                            n,
                            e,
                            eof
                        ));
                        break;
                    }
                    let flow = u32::from_le_bytes(m[4..8].try_into().unwrap());
                    if m[0] == 2 {
                        if CFG_CTL_IGNORE.load(SS) {
                            continue;
                        }
                        let mut g = FLOWS.lock().unwrap();
                        if let Some(f) = g.get_mut(&flow) {
                            f.aborted_ctl = true;
                            f.aborted_t = now_us();
                            CTL_ABORT_KNOWN.fetch_add(1, SS);
                        } else {
                            CTL_DISCARDED.fetch_add(1, SS);
                        }
                    } else {
                        CTL_UNEXPECTED.fetch_add(1, SS);
                    }
                }
                *SESSION.lock().unwrap() = None;
                // session loss aborts every flow the guest holds
                if !CFG_CTL_IGNORE.load(SS) {
                    let t = now_us();
                    for (_, f) in FLOWS.lock().unwrap().iter_mut() {
                        if !f.aborted_ctl {
                            f.aborted_ctl = true;
                            f.aborted_t = t;
                        }
                    }
                }
                unsafe { libc::close(fd) };
            }
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------- application (the C side)
fn app_loop() {
    let l = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    let one = 1i32;
    unsafe { libc::setsockopt(l, libc::SOL_SOCKET, libc::SO_REUSEADDR, &one as *const _ as *const _, 4) };
    let mut a: libc::sockaddr_in = unsafe { mem::zeroed() };
    a.sin_family = libc::AF_INET as _;
    a.sin_port = APP_PORT.to_be();
    a.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    unsafe {
        if libc::bind(l, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) < 0 {
            println!("GUEST_APP_BIND_FAIL errno={}", errno());
            return;
        }
        libc::listen(l, 4096);
    }
    loop {
        let (c, cport) = match accept_inet(l) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let _ = thread::Builder::new().stack_size(128 * 1024).spawn(move || {
            let t0 = now_us();
            set_rcvtimeo(c, 4000);
            let mut total = 0usize;
            let mut first = String::new();
            let mut first_t = 0;
            loop {
                let mut b = [0u8; 4096];
                let n = unsafe { libc::recv(c, b.as_mut_ptr() as *mut _, b.len(), 0) };
                let ev = if n > 0 {
                    total += n as usize;
                    "bytes".to_string()
                } else if n == 0 {
                    "eof".to_string()
                } else {
                    let e = errno();
                    if e == libc::ECONNRESET {
                        "reset".to_string()
                    } else if e == libc::EAGAIN {
                        "timeout".to_string()
                    } else {
                        format!("err{}", e)
                    }
                };
                if first.is_empty() {
                    first = ev.clone();
                    first_t = now_us();
                }
                if n <= 0 {
                    rpt(&format!(
                        "{{\"k\":\"app\",\"c_port\":{},\"accepted_t\":{},\"first\":\"{}\",\"first_t\":{},\"end\":\"{}\",\"end_t\":{},\"bytes\":{}}}",
                        cport,
                        t0,
                        first,
                        first_t,
                        ev,
                        now_us(),
                        total
                    ));
                    break;
                }
            }
            unsafe { libc::close(c) };
        });
    }
}

// ---------------------------------------------------------------- flow worker (the guest-flow owner)
fn tcp_connect_c() -> io::Result<(RawFd, u16)> {
    unsafe {
        let c = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        set_linger0(c);
        let mut a: libc::sockaddr_in = mem::zeroed();
        a.sin_family = libc::AF_INET as _;
        a.sin_port = APP_PORT.to_be();
        a.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
        if libc::connect(c, &a as *const _ as *const libc::sockaddr, mem::size_of_val(&a) as u32) < 0 {
            let e = io::Error::last_os_error();
            libc::close(c);
            return Err(e);
        }
        let mut la: libc::sockaddr_in = mem::zeroed();
        let mut ll = mem::size_of_val(&la) as u32;
        libc::getsockname(c, &mut la as *mut _ as *mut libc::sockaddr, &mut ll);
        Ok((c, u16::from_be(la.sin_port)))
    }
}

fn worker(vg: RawFd, label: String, hold_ms: u64, paired_blind: bool, ctl_ignore: bool) {
    let t_accept = now_us();
    let rev0 = poll1(vg, libc::POLLIN | libc::POLLRDHUP, 0);
    let (pk0, pk0e) = peek(vg);
    set_rcvtimeo(vg, 2000);
    let mut req = [0u8; 16];
    let (req_n, req_err, req_eof) = read_full(vg, &mut req);
    let flow = if req_n == 16 && &req[0..4] == b"RQ24" { Some(u32::from_le_bytes(req[4..8].try_into().unwrap())) } else { None };
    if let Some(f) = flow {
        FLOWS.lock().unwrap().insert(f, FlowSt::default());
    }
    let (c, c_port) = match tcp_connect_c() {
        Ok(x) => x,
        Err(e) => {
            rpt(&format!(
                "{{\"k\":\"flow\",\"label\":\"{}\",\"trial\":{},\"error\":\"connect_c {}\"}}",
                label,
                flow.map(|f| f as i64).unwrap_or(-1),
                e
            ));
            unsafe { libc::close(vg) };
            return;
        }
    };
    let t_c = now_us();
    rpt(&format!("{{\"k\":\"stage\",\"label\":\"{}\",\"trial\":{},\"t\":{}}}", label, flow.map(|f| f as i64).unwrap_or(-1), t_c));
    // hold (scripted); watch V_g and the control-abort flag
    let mut vg_event_t: Option<u64> = None;
    let mut vg_event_rev: i16 = 0;
    let mut abort_ctl_t: Option<u64> = None;
    let hold_until = Instant::now() + Duration::from_millis(hold_ms);
    while Instant::now() < hold_until {
        let r = poll1(vg, libc::POLLIN | libc::POLLRDHUP, 5);
        if r != 0 && vg_event_t.is_none() {
            vg_event_t = Some(now_us());
            vg_event_rev = r;
        }
        if let Some(f) = flow {
            if !ctl_ignore {
                if let Some(st) = FLOWS.lock().unwrap().get(&f) {
                    if st.aborted_ctl && abort_ctl_t.is_none() {
                        abort_ctl_t = Some(st.aborted_t);
                    }
                }
            }
        }
    }
    if let Some(f) = flow {
        if !ctl_ignore && abort_ctl_t.is_none() {
            if let Some(st) = FLOWS.lock().unwrap().get(&f) {
                if st.aborted_ctl {
                    abort_ctl_t = Some(st.aborted_t);
                }
            }
        }
    }
    let mut paired_sent = false;
    if paired_blind {
        if let Some(f) = flow {
            paired_sent = send_ctl(3, f);
        }
    }
    // observe V_g
    let rev1 = poll1(vg, libc::POLLIN | libc::POLLRDHUP, 0);
    let (pk1, pk1e) = peek(vg);
    set_rcvtimeo(vg, 300);
    let t_rd0 = now_us();
    let mut b1 = [0u8; 1];
    let rd = unsafe { libc::recv(vg, b1.as_mut_ptr() as *mut _, 1, 0) };
    let rd_err = if rd < 0 { errno() } else { 0 };
    let rd_dt = now_us() - t_rd0;
    let wr = unsafe { libc::send(vg, b"x".as_ptr() as *const _, 1, libc::MSG_NOSIGNAL) };
    let wr_err = if wr < 0 { errno() } else { 0 };
    let soe = so_error(vg);
    let ended = rd == 0 || (rd < 0 && rd_err != libc::EAGAIN && rd_err != libc::EWOULDBLOCK);
    let decision;
    let mut fwd_bytes = 0usize;
    let t_dec;
    if abort_ctl_t.is_some() || ended {
        // tear down V_g and C (C closes with RST: SO_LINGER{1,0})
        unsafe {
            libc::close(c);
            libc::close(vg);
        }
        t_dec = now_us();
        decision = if abort_ctl_t.is_some() { "teardown_ctl" } else { "teardown_vg" };
    } else {
        if !paired_sent {
            if let Some(f) = flow {
                paired_sent = send_ctl(3, f);
            }
        }
        // route V_g -> C for up to 2.5 s (positive control)
        if rd > 0 && write_all(c, &b1).is_ok() {
            fwd_bytes += 1;
        }
        let end = Instant::now() + Duration::from_millis(2500);
        set_rcvtimeo(vg, 100);
        let mut ended_late = false;
        while Instant::now() < end {
            let mut b = [0u8; 4096];
            let n = unsafe { libc::recv(vg, b.as_mut_ptr() as *mut _, b.len(), 0) };
            if n > 0 {
                if write_all(c, &b[..n as usize]).is_ok() {
                    fwd_bytes += n as usize;
                }
            } else if n == 0 || (n < 0 && errno() != libc::EAGAIN) {
                ended_late = true;
                break;
            }
        }
        unsafe {
            libc::close(c);
            libc::close(vg);
        }
        t_dec = now_us();
        decision = if ended_late { "routed_then_ended" } else { "routed_timeout" };
    }
    if let Some(f) = flow {
        FLOWS.lock().unwrap().remove(&f);
    }
    rpt(&format!(
        "{{\"k\":\"flow\",\"label\":\"{}\",\"trial\":{},\"t_accept\":{},\"t_c\":{},\"t_dec\":{},\"c_port\":{},\"hold_ms\":{},\
\"pre\":{{\"rev\":{},\"peek_rc\":{},\"peek_err\":{}}},\"req\":{{\"n\":{},\"err\":{},\"eof\":{}}},\
\"hold_obs\":{{\"vg_event_t\":{},\"vg_event_rev\":{},\"abort_ctl_t\":{}}},\
\"obs\":{{\"rev\":{},\"peek_rc\":{},\"peek_err\":{},\"rd_rc\":{},\"rd_err\":{},\"rd_dt_us\":{},\"wr_rc\":{},\"wr_err\":{},\"so_err\":{}}},\
\"paired_sent\":{},\"fwd_bytes\":{},\"decision\":\"{}\"}}",
        label,
        flow.map(|f| f as i64).unwrap_or(-1),
        t_accept,
        t_c,
        t_dec,
        c_port,
        hold_ms,
        rev0,
        pk0,
        pk0e,
        req_n,
        req_err,
        req_eof,
        vg_event_t.map(|v| v as i64).unwrap_or(-1),
        vg_event_rev,
        abort_ctl_t.map(|v| v as i64).unwrap_or(-1),
        rev1,
        pk1,
        pk1e,
        rd,
        rd_err,
        rd_dt,
        wr,
        wr_err,
        soe,
        paired_sent,
        fwd_bytes,
        decision
    ));
}

fn flow_acceptor() {
    let l = loop {
        match vsock_listen(FLOW_PORT, 4096) {
            Ok(l) => break l,
            Err(e) => {
                println!("GUEST_FLOW_LISTEN_FAIL {}", e);
                thread::sleep(Duration::from_millis(200));
            }
        }
    };
    loop {
        // While HOLD is set accept() is never called, so connections stay in the kernel accept queue.
        if HOLD.load(SS) {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        if poll1(l, libc::POLLIN, 5) & libc::POLLIN == 0 || HOLD.load(SS) {
            continue;
        }
        let vg = match accept(l) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let label = LABEL.lock().unwrap().clone();
        let hold = CFG_HOLD_MS.load(SS);
        let blind = CFG_PAIRED_BLIND.load(SS);
        let ign = CFG_CTL_IGNORE.load(SS);
        let _ = thread::Builder::new().stack_size(128 * 1024).spawn(move || worker(vg, label, hold, blind, ign));
    }
}

// ---------------------------------------------------------------- command channel
fn cmd_conn(fd: RawFd) {
    let mut buf = Vec::new();
    loop {
        let mut b = [0u8; 256];
        let n = unsafe { libc::recv(fd, b.as_mut_ptr() as *mut _, b.len(), 0) };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&b[..n as usize]);
        while let Some(p) = buf.iter().position(|&c| c == b'\n') {
            let line = String::from_utf8_lossy(&buf[..p]).to_string();
            buf.drain(..=p);
            let mut it = line.split_whitespace();
            let cmd = it.next().unwrap_or("");
            let reply = match cmd {
                "ping" => "pong".to_string(),
                "cfg" => {
                    for kv in it {
                        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                        match k {
                            "label" => *LABEL.lock().unwrap() = v.to_string(),
                            "hold_ms" => CFG_HOLD_MS.store(v.parse().unwrap_or(0), SS),
                            "paired_blind" => CFG_PAIRED_BLIND.store(v == "1", SS),
                            "ctl_ignore" => CFG_CTL_IGNORE.store(v == "1", SS),
                            _ => {}
                        }
                    }
                    "ok".to_string()
                }
                "hold" => {
                    HOLD.store(it.next() == Some("on"), SS);
                    "ok".to_string()
                }
                "reset" => {
                    FLOWS.lock().unwrap().clear();
                    CTL_DISCARDED.store(0, SS);
                    CTL_ABORT_KNOWN.store(0, SS);
                    CTL_UNEXPECTED.store(0, SS);
                    "ok".to_string()
                }
                "counters" => format!(
                    "{{\"k\":\"counters\",\"ctl_discarded\":{},\"ctl_abort_known\":{},\"ctl_unexpected\":{},\"session_ups\":{},\"flows_tracked\":{}}}",
                    CTL_DISCARDED.load(SS),
                    CTL_ABORT_KNOWN.load(SS),
                    CTL_UNEXPECTED.load(SS),
                    SESSION_UPS.load(SS),
                    FLOWS.lock().unwrap().len()
                ),
                "diag" => diag_json(),
                "poweroff" => {
                    let _ = write_all(fd, b"bye\n");
                    unsafe {
                        libc::sync();
                        libc::reboot(libc::RB_POWER_OFF);
                    }
                    "bye".to_string()
                }
                _ => "err unknown".to_string(),
            };
            let _ = write_all(fd, format!("{}\n", reply).as_bytes());
        }
    }
    unsafe { libc::close(fd) };
}

fn cmd_loop() {
    let l = loop {
        match vsock_listen(CMD_PORT, 8) {
            Ok(l) => break l,
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    };
    loop {
        if let Ok(c) = accept(l) {
            thread::spawn(move || cmd_conn(c));
        }
    }
}

fn main() {
    START.set(Instant::now()).ok();
    // 1000 flows x (V_g + C + the application's accepted socket) exceeds the kernel's default 1024.
    let rl = libc::rlimit { rlim_cur: 65536, rlim_max: 65536 };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rl) } < 0 {
        println!("GUEST_RLIMIT_FAIL errno={}", errno());
    }
    lo_up();
    println!("GUEST_LOCAL_CID {}", local_cid());
    // report channel
    let mut tries = 0;
    let rfd = loop {
        match vsock_connect(2, HOST_RPT_PORT) {
            Ok(fd) => break fd,
            Err(e) => {
                tries += 1;
                if tries % 20 == 1 {
                    println!("GUEST_RPT_CONNECT_RETRY {}", e);
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    };
    *RPT.lock().unwrap() = Some(rfd);
    let mut uts: libc::utsname = unsafe { mem::zeroed() };
    unsafe { libc::uname(&mut uts) };
    let rel: String = uts.release.iter().take_while(|&&c| c != 0).map(|&c| c as u8 as char).collect();
    rpt(&format!("{{\"k\":\"hello\",\"uname_r\":\"{}\",\"local_cid\":{},\"t\":{}}}", rel, local_cid(), now_us()));
    thread::spawn(app_loop);
    thread::spawn(control_loop);
    thread::spawn(cmd_loop);
    flow_acceptor();
}
