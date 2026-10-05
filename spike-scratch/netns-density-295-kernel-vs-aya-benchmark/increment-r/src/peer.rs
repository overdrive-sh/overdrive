use std::{
    fs, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::atomic::{fence, Ordering},
    time::{Duration, Instant},
};
const SIZE: usize = 256 * 1024;
const N: u16 = 8;
const BUF: u64 = 0x8000;
pub(crate) fn ioctl<T>(fd: i32, req: u64, p: &T) {
    let rc = unsafe { libc::ioctl(fd, req as libc::c_ulong, p as *const T) };
    assert_eq!(rc, 0, "ioctl {req:x}: {}", io::Error::last_os_error());
}
#[repr(C)]
struct State {
    index: u32,
    num: u32,
}
#[repr(C)]
struct Addr {
    index: u32,
    flags: u32,
    desc: u64,
    used: u64,
    avail: u64,
    log: u64,
}
#[repr(C)]
struct VFile {
    index: u32,
    fd: i32,
}
#[repr(C)]
struct Memory {
    n: u32,
    pad: u32,
    gpa: u64,
    size: u64,
    user: u64,
    flags: u64,
}
pub(crate) struct Peer {
    pub(crate) typ: u16,
    pub(crate) fd: OwnedFd,
    pub(crate) mem: *mut u8,
    kicks: Vec<OwnedFd>,
    _calls: Vec<OwnedFd>,
    pub(crate) cid: u32,
    pub(crate) port: u32,
    rx_av: u16,
    rx_used: u16,
    tx_av: u16,
    tx_used: u16,
    flows: std::collections::HashMap<u32, (u16, u32, u32)>,
    pub(crate) reverse_port: Option<u32>,
    packets: u64,
    rx_pending: bool,
}
unsafe impl Send for Peer {} // Exclusive shard owns this mapping and its virtqueues.
impl Peer {
    pub(crate) fn rx_call_fd(&self) -> i32 {
        self._calls[0].as_raw_fd()
    }
    fn wait_call(&self, q: usize, until: Instant) {
        let ms = (until.saturating_duration_since(Instant::now()).as_millis().min(1000)) as i32;
        let mut p =
            libc::pollfd { fd: self._calls[q].as_raw_fd(), events: libc::POLLIN, revents: 0 };
        unsafe {
            libc::poll(&mut p, 1, ms.max(1));
            let mut x = 0u64;
            libc::read(p.fd, &mut x as *mut _ as *mut _, 8);
        }
    }
    pub(crate) fn new(cid: u32, port: u32, typ: u16) -> Self {
        let fd = fs::OpenOptions::new().read(true).write(true).open("/dev/vhost-vsock").unwrap();
        let fd: OwnedFd = fd.into();
        let mem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        } as *mut u8;
        assert_ne!(mem as isize, -1);
        let mut offered = 0u64;
        assert_eq!(
            unsafe { libc::ioctl(fd.as_raw_fd(), 0x8008af00u64 as libc::c_ulong, &mut offered) },
            0
        );
        ioctl(fd.as_raw_fd(), 0xaf01, &0u64);
        ioctl(fd.as_raw_fd(), 0x4008af00, &((1u64 << 32) | (1u64 << 1)));
        ioctl(
            fd.as_raw_fd(),
            0x4008af03,
            &Memory { n: 1, pad: 0, gpa: 0, size: SIZE as u64, user: mem as u64, flags: 0 },
        );
        let mut kicks = vec![];
        let mut calls = vec![];
        for q in 0..2u32 {
            ioctl(fd.as_raw_fd(), 0x4008af10, &State { index: q, num: N as u32 });
            ioctl(fd.as_raw_fd(), 0x4008af12, &State { index: q, num: 0 });
            let b = q as u64 * 0x4000;
            ioctl(
                fd.as_raw_fd(),
                0x4028af11,
                &Addr {
                    index: q,
                    flags: 0,
                    desc: mem as u64 + b,
                    avail: mem as u64 + b + 0x1000,
                    used: mem as u64 + b + 0x1800,
                    log: 0,
                },
            );
            let kick = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_NONBLOCK)) };
            let call = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_NONBLOCK)) };
            ioctl(fd.as_raw_fd(), 0x4008af20, &VFile { index: q, fd: kick.as_raw_fd() });
            ioctl(fd.as_raw_fd(), 0x4008af21, &VFile { index: q, fd: call.as_raw_fd() });
            kicks.push(kick);
            calls.push(call);
        }
        ioctl(fd.as_raw_fd(), 0x4008af60, &(cid as u64));
        ioctl(fd.as_raw_fd(), 0x4004af61, &1i32);
        let p = Self {
            typ,
            fd,
            mem,
            kicks,
            _calls: calls,
            cid,
            port,
            rx_av: 0,
            rx_used: 0,
            tx_av: 0,
            tx_used: 0,
            flows: [(port, (typ, 29500, 0))].into_iter().collect(),
            reverse_port: None,
            packets: 0,
            rx_pending: false,
        };
        p.desc(0, BUF, 44, 3, 1);
        p.desc(16, BUF + 0x100, 65536, 2, 0);
        p
    }
    pub(crate) fn write(&self, a: u64, b: &[u8]) {
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), self.mem.add(a as usize), b.len()) }
    }
    pub(crate) fn read(&self, a: u64, n: usize) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(self.mem.add(a as usize), n).to_vec() }
    }
    pub(crate) fn w16(&self, a: u64, n: u16) {
        unsafe { std::ptr::write_volatile(self.mem.add(a as usize) as *mut u16, n.to_le()) }
    }
    pub(crate) fn r16(&self, a: u64) -> u16 {
        unsafe { u16::from_le(std::ptr::read_volatile(self.mem.add(a as usize) as *const u16)) }
    }
    pub(crate) fn desc(&self, a: u64, addr: u64, len: u32, flags: u16, next: u16) {
        let mut b = vec![];
        b.extend(addr.to_le_bytes());
        b.extend(len.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend(next.to_le_bytes());
        self.write(a, &b)
    }
    pub(crate) fn kick(&self, q: usize) {
        let n = 1u64;
        assert_eq!(
            unsafe { libc::write(self.kicks[q].as_raw_fd(), &n as *const u64 as *const _, 8) },
            8
        )
    }
    pub(crate) fn tx(&mut self, op: u16, flags: u32, b: &[u8]) {
        self.tx_flow(self.port, op, flags, b)
    }
    fn tx_flow(&mut self, port: u32, op: u16, flags: u32, b: &[u8]) {
        let (typ, host_port, fwd) = self.flows[&port];
        let mut h = vec![];
        h.extend((self.cid as u64).to_le_bytes());
        h.extend(2u64.to_le_bytes());
        h.extend(port.to_le_bytes());
        h.extend(host_port.to_le_bytes());
        h.extend((b.len() as u32).to_le_bytes());
        h.extend(typ.to_le_bytes());
        h.extend(op.to_le_bytes());
        h.extend(flags.to_le_bytes());
        h.extend(65536u32.to_le_bytes());
        h.extend(fwd.to_le_bytes());
        self.write(0x20000, &h);
        self.write(0x20100, b);
        self.desc(0x4000, 0x20000, 44, 1, 1);
        self.desc(0x4010, 0x20100, b.len() as u32, 0, 0);
        self.w16(0x5004 + (self.tx_av % N) as u64 * 2, 0);
        self.tx_av = self.tx_av.wrapping_add(1);
        fence(Ordering::SeqCst);
        self.w16(0x5002, self.tx_av);
        self.kick(1);
        let until = Instant::now() + Duration::from_secs(4);
        while self.r16(0x5802) == self.tx_used {
            assert!(Instant::now() < until, "TX timeout CID {}", self.cid);
            self.wait_call(1, until);
        }
        fence(Ordering::SeqCst);
        self.tx_used = self.tx_used.wrapping_add(1);
    }
    pub(crate) fn try_rx(&mut self, timeout: Duration) -> Option<(u16, u32, Vec<u8>)> {
        if !self.rx_pending {
            self.w16(0x1004 + (self.rx_av % N) as u64 * 2, 0);
            self.rx_av = self.rx_av.wrapping_add(1);
            fence(Ordering::SeqCst);
            self.w16(0x1002, self.rx_av);
            self.kick(0);
            self.rx_pending = true;
        }
        let until = Instant::now() + timeout;
        while self.r16(0x1802) == self.rx_used {
            if Instant::now() >= until {
                return None;
            }
            self.wait_call(0, until);
        }
        fence(Ordering::SeqCst);
        self.rx_used = self.rx_used.wrapping_add(1);
        self.rx_pending = false;
        let h = self.read(BUF, 44);
        assert_eq!(u64::from_le_bytes(h[8..16].try_into().unwrap()), self.cid as u64);
        let port = u32::from_le_bytes(h[20..24].try_into().unwrap());
        let flow = *self.flows.get(&port).expect("unowned guest destination port");
        assert_eq!(u16::from_le_bytes(h[28..30].try_into().unwrap()), flow.0);
        assert_eq!(u32::from_le_bytes(h[16..20].try_into().unwrap()), flow.1);
        let n = u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;
        assert!(n <= 65536);
        let op = u16::from_le_bytes(h[30..32].try_into().unwrap());
        let flags = u32::from_le_bytes(h[32..36].try_into().unwrap());
        let b = self.read(BUF + 0x100, n);

        self.flows.get_mut(&port).unwrap().2 = flow.2.wrapping_add(n as u32);
        if op == 5 { assert_eq!(port, self.port, "payload arrived on reverse-only flow"); }
        self.packets += 1;
        if n > 0 {
            self.tx_flow(port, 6, 0, &[]);
        }
        Some((op, flags, b))
    }
    pub(crate) fn rx(&mut self) -> (u16, u32, Vec<u8>) {
        self.try_rx(Duration::from_secs(4)).expect("RX timed out")
    }
    pub(crate) fn connect(&mut self) {
        self.tx(1, 0, &[]);
        loop {
            let (op, _, b) = self.rx();
            if op == 2 {
                assert!(b.is_empty());
                break;
            }
            assert_ne!(op, 3, "connect rejected")
        }
    }
    pub(crate) fn connect_reverse(&mut self) {
        let port = self.port + 100000;
        assert!(self.flows.insert(port, (2, 29501, 0)).is_none());
        self.reverse_port = Some(port);
        self.tx_flow(port, 1, 0, &[]);
        loop { let (op, _, data) = self.rx(); if op == 2 { assert!(data.is_empty()); break; } assert_eq!(op, 6); }
    }
    pub(crate) fn send_reverse(&mut self, data: &[u8]) {
        self.tx_flow(self.reverse_port.unwrap(), 5, 1, data)
    }
    pub(crate) fn reset_reverse(&mut self) {
        if let Some(port) = self.reverse_port { self.tx_flow(port, 3, 0, &[]) }
    }
    pub(crate) fn bytes(&mut self, b: &[u8], split: usize) {
        for c in b.chunks(split) {
            self.tx(5, 0, c)
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        ioctl(self.fd.as_raw_fd(), 0x4004af61, &0i32);
        unsafe {
            libc::munmap(self.mem as *mut _, SIZE);
        }
    }
}
