use std::fs::{self, File};
use std::io;
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use aya::maps::{HashMap as AyaHashMap, PerCpuArray};
use aya::programs::{
    CgroupAttachMode, CgroupSkb, CgroupSkbAttachType, CgroupSock, SockOps, SocketFilter,
};
use aya::EbpfLoader;

const TCP_FASTOPEN: i32 = 23;
const READY: u32 = 2;
const REJECT: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Meta {
    application_cookie: u64,
    destination_ip: u32,
    destination_port: u16,
    pad: u16,
}

unsafe impl aya::Pod for Meta {}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildState {
    lock: u32,
    state: u32,
    meta: Meta,
}

unsafe impl aya::Pod for ChildState {}

// ------------------------------------------------------------------ raw bpf(2)

fn bpf(cmd: libc::c_long, attr: &mut [u8; 128]) -> Result<i64, i32> {
    let result = unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), attr.len()) };
    if result < 0 {
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
    } else {
        Ok(result)
    }
}

fn put32(attr: &mut [u8; 128], offset: usize, value: u32) {
    attr[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn put64(attr: &mut [u8; 128], offset: usize, value: u64) {
    attr[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}

/// Minimal BTF: u32/u64 keys, a child value whose first field is one
/// `bpf_spin_lock`, and the separate lock-free active-storage value.
fn correlation_btf_blob() -> Vec<u8> {
    let mut strings = vec![0u8];
    let mut add_string = |name: &[u8]| {
        let offset = strings.len() as u32;
        strings.extend_from_slice(name);
        strings.push(0);
        offset
    };
    let u32_name = add_string(b"u32");
    let u64_name = add_string(b"u64");
    let spin_name = add_string(b"bpf_spin_lock");
    let val_name = add_string(b"val");
    let child_name = add_string(b"child_state");
    let lock_name = add_string(b"lock");
    let active_name = add_string(b"active");
    let mut types = Vec::new();
    // 1: u32
    types.extend_from_slice(&u32_name.to_ne_bytes());
    types.extend_from_slice(&(1u32 << 24).to_ne_bytes()); // BTF_KIND_INT
    types.extend_from_slice(&4u32.to_ne_bytes());
    types.extend_from_slice(&32u32.to_ne_bytes());
    // 2: u64
    types.extend_from_slice(&u64_name.to_ne_bytes());
    types.extend_from_slice(&(1u32 << 24).to_ne_bytes());
    types.extend_from_slice(&8u32.to_ne_bytes());
    types.extend_from_slice(&64u32.to_ne_bytes());
    // 3: struct bpf_spin_lock { u32 val; }
    types.extend_from_slice(&spin_name.to_ne_bytes());
    types.extend_from_slice(&((4u32 << 24) | 1).to_ne_bytes());
    types.extend_from_slice(&4u32.to_ne_bytes());
    types.extend_from_slice(&val_name.to_ne_bytes());
    types.extend_from_slice(&1u32.to_ne_bytes());
    types.extend_from_slice(&0u32.to_ne_bytes());
    // 4: child_state, with the lock at byte zero. Remaining bytes are plain.
    types.extend_from_slice(&child_name.to_ne_bytes());
    types.extend_from_slice(&((4u32 << 24) | 1).to_ne_bytes());
    types.extend_from_slice(&24u32.to_ne_bytes());
    types.extend_from_slice(&lock_name.to_ne_bytes());
    types.extend_from_slice(&3u32.to_ne_bytes());
    types.extend_from_slice(&0u32.to_ne_bytes());
    // 5: lock-free active state.
    types.extend_from_slice(&active_name.to_ne_bytes());
    types.extend_from_slice(&(4u32 << 24).to_ne_bytes()); // BTF_KIND_STRUCT, vlen=0
    types.extend_from_slice(&24u32.to_ne_bytes());

    let mut blob = Vec::new();
    blob.extend_from_slice(&0xeB9Fu16.to_ne_bytes());
    blob.push(1);
    blob.push(0);
    blob.extend_from_slice(&24u32.to_ne_bytes());
    blob.extend_from_slice(&0u32.to_ne_bytes());
    blob.extend_from_slice(&(types.len() as u32).to_ne_bytes());
    blob.extend_from_slice(&(types.len() as u32).to_ne_bytes());
    blob.extend_from_slice(&(strings.len() as u32).to_ne_bytes());
    blob.extend_from_slice(&types);
    blob.extend_from_slice(&strings);
    blob
}

fn load_correlation_btf() -> Result<OwnedFd, i32> {
    let blob = correlation_btf_blob();
    let mut attr = [0u8; 128];
    put64(&mut attr, 0, blob.as_ptr() as u64);
    put32(&mut attr, 16, blob.len() as u32);
    bpf(18, &mut attr).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn create_active_storage(btf: RawFd) -> Result<OwnedFd, i32> {
    let mut attr = [0u8; 128];
    put32(&mut attr, 0, 24); // BPF_MAP_TYPE_SK_STORAGE
    put32(&mut attr, 4, 4);
    put32(&mut attr, 8, 24);
    put32(&mut attr, 12, 0);
    put32(&mut attr, 16, 1); // BPF_F_NO_PREALLOC
    for (index, byte) in b"ACTIVE".iter().copied().enumerate() {
        attr[28 + index] = byte;
    }
    put32(&mut attr, 48, btf as u32);
    put32(&mut attr, 52, 1);
    put32(&mut attr, 56, 5);
    bpf(0, &mut attr).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn create_child_map(btf: RawFd) -> Result<OwnedFd, i32> {
    let mut attr = [0u8; 128];
    put32(&mut attr, 0, 1); // BPF_MAP_TYPE_HASH
    put32(&mut attr, 4, 8);
    put32(&mut attr, 8, 24);
    put32(&mut attr, 12, 8_194);
    for (index, byte) in b"CHILD".iter().copied().enumerate() {
        attr[28 + index] = byte;
    }
    put32(&mut attr, 48, btf as u32);
    put32(&mut attr, 52, 2);
    put32(&mut attr, 56, 4);
    bpf(0, &mut attr).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn pin_object(fd: RawFd, path: &str) -> Result<(), i32> {
    let path = std::ffi::CString::new(path).unwrap();
    let mut attr = [0u8; 128];
    put64(&mut attr, 0, path.as_ptr() as u64);
    put32(&mut attr, 8, fd as u32);
    bpf(6, &mut attr).map(|_| ())
}

fn cvt(rc: i32) -> io::Result<i32> {
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

fn socket() -> io::Result<OwnedFd> {
    let fd =
        cvt(unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn set_int(fd: RawFd, level: i32, option: i32, value: i32) {
    cvt(unsafe {
        libc::setsockopt(
            fd,
            level,
            option,
            (&value as *const i32).cast(),
            size_of::<i32>() as libc::socklen_t,
        )
    })
    .unwrap_or_else(|error| panic!("setsockopt({level}, {option}, {value}): {error}"));
}

fn set_reset_close(fd: RawFd) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    cvt(unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&linger as *const libc::linger).cast(),
            size_of::<libc::linger>() as libc::socklen_t,
        )
    })
    .expect("set reset close");
}

fn duplicate(fd: RawFd) -> OwnedFd {
    let copied = cvt(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) }).expect("duplicate fd");
    unsafe { OwnedFd::from_raw_fd(copied) }
}

fn send_burst(fd: RawFd, count: usize) {
    set_int(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY, 1);
    let byte = [0x5au8];
    for _ in 0..count {
        let written =
            unsafe { libc::send(fd, byte.as_ptr().cast(), byte.len(), libc::MSG_NOSIGNAL) };
        if written < 0 {
            return;
        }
        assert_eq!(written, 1, "one stress byte");
        thread::yield_now();
    }
}

struct FinalAckGate;

impl FinalAckGate {
    fn install() -> Self {
        Self::run(&["qdisc", "add", "dev", "lo", "root", "handle", "1:", "prio", "bands", "3"]);
        let gate = Self;
        Self::run(&[
            "qdisc", "add", "dev", "lo", "parent", "1:3", "handle", "30:", "netem", "delay",
            "1000ms", "limit", "1000",
        ]);
        gate
    }

    fn select(fd: RawFd) {
        set_int(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, 0x0001_0003);
    }

    fn remove(self) {
        Self::run(&["qdisc", "del", "dev", "lo", "root"]);
        std::mem::forget(self);
    }

    fn run(args: &[&str]) {
        let output = Command::new("/usr/sbin/tc").args(args).output().expect("run tc");
        assert!(
            output.status.success(),
            "tc {}: status={:?}, stderr={}",
            args.join(" "),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for FinalAckGate {
    fn drop(&mut self) {
        let _ = Command::new("/usr/sbin/tc").args(["qdisc", "del", "dev", "lo", "root"]).output();
    }
}

fn sockaddr(addr: SocketAddrV4) -> libc::sockaddr_in {
    libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: addr.port().to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(addr.ip().octets()) },
        sin_zero: [0; 8],
    }
}

fn bind(fd: RawFd, addr: SocketAddrV4) -> io::Result<()> {
    let addr = sockaddr(addr);
    cvt(unsafe {
        libc::bind(
            fd,
            (&addr as *const libc::sockaddr_in).cast(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

fn local(fd: RawFd) -> io::Result<SocketAddrV4> {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;
    cvt(unsafe { libc::getsockname(fd, (&mut addr as *mut libc::sockaddr_in).cast(), &mut len) })?;
    Ok(SocketAddrV4::new(
        Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
        u16::from_be(addr.sin_port),
    ))
}

fn cookie(fd: RawFd) -> io::Result<u64> {
    let mut value = 0u64;
    let mut len = size_of::<u64>() as libc::socklen_t;
    cvt(unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_COOKIE,
            (&mut value as *mut u64).cast(),
            &mut len,
        )
    })?;
    Ok(value)
}

fn listener(fast_open: bool) -> io::Result<OwnedFd> {
    let fd = socket()?;
    bind(fd.as_raw_fd(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
    if fast_open {
        let backlog = 1024i32;
        cvt(unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::IPPROTO_TCP,
                TCP_FASTOPEN,
                (&backlog as *const i32).cast(),
                size_of::<i32>() as libc::socklen_t,
            )
        })?;
    }
    cvt(unsafe { libc::listen(fd.as_raw_fd(), 4096) })?;
    Ok(fd)
}

fn connect(fd: RawFd, addr: SocketAddrV4) -> io::Result<()> {
    let addr = sockaddr(addr);
    cvt(unsafe {
        libc::connect(
            fd,
            (&addr as *const libc::sockaddr_in).cast(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

fn fast_open_send(fd: RawFd, addr: SocketAddrV4, data: &[u8]) -> io::Result<usize> {
    let addr = sockaddr(addr);
    let written = unsafe {
        libc::sendto(
            fd,
            data.as_ptr().cast(),
            data.len(),
            libc::MSG_FASTOPEN,
            (&addr as *const libc::sockaddr_in).cast(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if written < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(written as usize)
    }
}

fn accept(fd: RawFd) -> io::Result<OwnedFd> {
    let child = cvt(unsafe {
        libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC)
    })?;
    Ok(unsafe { OwnedFd::from_raw_fd(child) })
}

fn read_counters(map: &PerCpuArray<aya::maps::MapData, u64>) -> Vec<u64> {
    (0..15)
        .map(|i| map.get(&i, 0).map(|values| values.iter().copied().sum()).unwrap_or(0))
        .collect()
}

fn root_socket_bypasses_private_ingress(
    child: &mut AyaHashMap<aya::maps::MapData, u64, ChildState>,
    counters: &PerCpuArray<aya::maps::MapData, u64>,
) {
    let before = read_counters(counters);
    let listener = listener(false).expect("root-cgroup scope-control listener");
    let target = local(listener.as_raw_fd()).expect("scope-control address");
    let client = socket().expect("scope-control client");
    connect(client.as_raw_fd(), target).expect("scope-control connect");
    let accepted = accept(listener.as_raw_fd()).expect("scope-control accept");
    let child_cookie = cookie(accepted.as_raw_fd()).expect("scope-control child cookie");
    let byte = [0xabu8];
    let written = unsafe {
        libc::send(client.as_raw_fd(), byte.as_ptr().cast(), byte.len(), libc::MSG_NOSIGNAL)
    };
    assert_eq!(written, 1, "scope-control send");
    let mut got = [0u8; 1];
    let read = unsafe { libc::recv(accepted.as_raw_fd(), got.as_mut_ptr().cast(), got.len(), 0) };
    assert_eq!(read, 1, "scope-control receive");
    assert_eq!(got, byte, "scope-control payload");
    thread::sleep(Duration::from_millis(25));
    let after = read_counters(counters);
    for index in [C_TFO_CANDIDATE_READY, C_TFO_CANDIDATE_REJECT, C_TERMINAL_NOOP] {
        assert_eq!(
            after[index], before[index],
            "root-cgroup socket must bypass private intake ingress counter {index}"
        );
    }
    let _ = child.remove(&child_cookie);
    println!("scope_control root_socket_bypassed_private_ingress=true child_cookie={child_cookie}");
}

const C_TFO_CANDIDATE_READY: usize = 12;
const C_TFO_CANDIDATE_REJECT: usize = 13;
const C_TERMINAL_NOOP: usize = 14;

fn tcpext(name: &str) -> u64 {
    let text = fs::read_to_string("/proc/net/netstat").expect("read netstat");
    let mut lines = text.lines();
    while let (Some(names), Some(values)) = (lines.next(), lines.next()) {
        if !names.starts_with("TcpExt:") || !values.starts_with("TcpExt:") {
            continue;
        }
        let keys = names.split_whitespace().skip(1);
        let vals = values.split_whitespace().skip(1);
        for (key, value) in keys.zip(vals) {
            if key == name {
                return value.parse().expect("netstat counter");
            }
        }
    }
    panic!("missing TcpExt counter {name}");
}

fn write_sysctl(path: &str, value: &str) {
    fs::write(path, value).unwrap_or_else(|err| panic!("write {path}={value}: {err}"));
}

fn ensure_cgroup(path: &str) {
    let root = Path::new("/sys/fs/cgroup");
    fs::create_dir_all(root).expect("create cgroup mountpoint");
    if !root.join("cgroup.controllers").is_file() {
        let source = b"none\0";
        let target = b"/sys/fs/cgroup\0";
        let fstype = b"cgroup2\0";
        let rc = unsafe {
            libc::mount(
                source.as_ptr().cast(),
                target.as_ptr().cast(),
                fstype.as_ptr().cast(),
                0,
                std::ptr::null(),
            )
        };
        assert_eq!(rc, 0, "mount cgroup2: {}", io::Error::last_os_error());
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => panic!("create probe cgroup {path}: {error}"),
    }
}

fn join_cgroup(path: &str) {
    fs::write(format!("{path}/cgroup.procs"), std::process::id().to_string())
        .unwrap_or_else(|error| panic!("join cgroup {path}: {error}"));
}

fn listener_in_cgroup(parent: &str, intake: &str, fast_open: bool) -> io::Result<OwnedFd> {
    join_cgroup(intake);
    let result = listener(fast_open);
    join_cgroup(parent);
    result
}

fn ensure_bpffs() {
    let root = Path::new("/sys/fs/bpf");
    fs::create_dir_all(root).expect("create bpffs mountpoint");
    let source = b"none\0";
    let target = b"/sys/fs/bpf\0";
    let fstype = b"bpf\0";
    let rc = unsafe {
        libc::mount(
            source.as_ptr().cast(),
            target.as_ptr().cast(),
            fstype.as_ptr().cast(),
            0,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        let error = io::Error::last_os_error();
        assert_eq!(error.raw_os_error(), Some(libc::EBUSY), "mount bpffs: {error}");
    }
}

fn wait_child(
    child: &AyaHashMap<aya::maps::MapData, u64, ChildState>,
    child_cookie: u64,
    expected_state: u32,
) -> ChildState {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(state) = child.get(&child_cookie, 0) {
            if state.state == expected_state {
                return state;
            }
        }
        assert!(
            Instant::now() < deadline,
            "child {child_cookie} did not reach state {expected_state}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn meta_for(application_cookie: u64, suffix: u8) -> Meta {
    Meta {
        application_cookie,
        destination_ip: u32::from_ne_bytes([203, 0, 113, suffix]),
        destination_port: 400u16 + u16::from(suffix),
        pad: 0,
    }
}

fn captured_connect(
    origin: &mut AyaHashMap<aya::maps::MapData, u64, Meta>,
    child: &AyaHashMap<aya::maps::MapData, u64, ChildState>,
    listener: RawFd,
    target: SocketAddrV4,
    suffix: u8,
) -> (u64, u64, Meta) {
    let client = socket().unwrap();
    let app_cookie = cookie(client.as_raw_fd()).unwrap();
    let expected = meta_for(app_cookie, suffix);
    origin.insert(app_cookie, expected, 0).unwrap();
    connect(client.as_raw_fd(), target).expect("connect captured client");
    drop(client); // immediate application release before userspace accept
    let accepted = accept(listener).expect("accept captured child");
    let child_cookie = cookie(accepted.as_raw_fd()).unwrap();
    let got = wait_child(child, child_cookie, READY);
    assert_eq!(got.meta, expected, "handshake ACK carries exact cookie and destination");
    (app_cookie, child_cookie, got.meta)
}

fn captured_fast_open(
    origin: &mut AyaHashMap<aya::maps::MapData, u64, Meta>,
    child: &AyaHashMap<aya::maps::MapData, u64, ChildState>,
    listener: RawFd,
    target: SocketAddrV4,
    suffix: u8,
    data: &[u8],
) -> (u64, u64, Meta) {
    let client = socket().unwrap();
    let app_cookie = cookie(client.as_raw_fd()).unwrap();
    let expected = meta_for(app_cookie, suffix);
    origin.insert(app_cookie, expected, 0).unwrap();
    assert_eq!(fast_open_send(client.as_raw_fd(), target, data).expect("MSG_FASTOPEN"), data.len());
    let accepted = accept(listener).expect("accept Fast Open child");
    let child_cookie = cookie(accepted.as_raw_fd()).unwrap();
    assert!(unsafe { libc::fcntl(client.as_raw_fd(), libc::F_GETFD) } >= 0);
    println!("fastopen_wait_open client_fd={} child_cookie={child_cookie}", client.as_raw_fd());
    let got = wait_child(child, child_cookie, READY);
    assert!(unsafe { libc::fcntl(client.as_raw_fd(), libc::F_GETFD) } >= 0);
    assert_eq!(got.meta, expected, "Fast Open final ACK carries exact metadata");
    let mut buf = vec![0u8; data.len()];
    let n = unsafe { libc::recv(accepted.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
    assert_eq!(n, data.len() as isize, "Fast Open data readable");
    assert_eq!(&buf, data, "Fast Open data exact");
    (app_cookie, child_cookie, got.meta)
}

fn direct_fast_open_rejected(
    child: &AyaHashMap<aya::maps::MapData, u64, ChildState>,
    listener: RawFd,
    target: SocketAddrV4,
    data: &[u8],
) -> u64 {
    let client = socket().unwrap();
    assert_eq!(
        fast_open_send(client.as_raw_fd(), target, data).expect("direct MSG_FASTOPEN"),
        data.len()
    );
    let accepted = accept(listener).expect("accept direct Fast Open child");
    let child_cookie = cookie(accepted.as_raw_fd()).unwrap();
    assert!(unsafe { libc::fcntl(client.as_raw_fd(), libc::F_GETFD) } >= 0);
    let got = wait_child(child, child_cookie, REJECT);
    assert_eq!(got.state, REJECT, "uncorrelated Fast Open child is rejected");
    assert!(unsafe { libc::fcntl(client.as_raw_fd(), libc::F_GETFD) } >= 0);
    println!(
        "fastopen_direct_rejected_open client_fd={} child_cookie={child_cookie}",
        client.as_raw_fd()
    );
    child_cookie
}

fn wait_missing(child: &AyaHashMap<aya::maps::MapData, u64, ChildState>, child_cookie: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.get(&child_cookie, 0).is_ok() {
        assert!(Instant::now() < deadline, "child {child_cookie} was not deleted");
        thread::yield_now();
    }
}

fn terminal_and_delete_are_monotonic(
    origin: &mut AyaHashMap<aya::maps::MapData, u64, Meta>,
    child: &mut AyaHashMap<aya::maps::MapData, u64, ChildState>,
    listener: RawFd,
    target: SocketAddrV4,
) {
    let gate = FinalAckGate::install();
    let client = socket().expect("race client");
    FinalAckGate::select(client.as_raw_fd());
    let app_cookie = cookie(client.as_raw_fd()).expect("race app cookie");
    let expected = meta_for(app_cookie, 21);
    origin.insert(app_cookie, expected, 0).expect("race origin");
    assert_eq!(
        fast_open_send(client.as_raw_fd(), target, b"race-syn").expect("race MSG_FASTOPEN"),
        8
    );
    let accepted = accept(listener).expect("accept race child");
    let child_cookie = cookie(accepted.as_raw_fd()).expect("race child cookie");
    let sender = duplicate(client.as_raw_fd());
    let burst = thread::spawn(move || send_burst(sender.as_raw_fd(), 128));
    let ready = wait_child(child, child_cookie, READY);
    assert_eq!(ready.meta, expected, "race terminal metadata");
    burst.join().expect("parallel sender");
    thread::sleep(Duration::from_millis(50));
    let still_ready = child.get(&child_cookie, 0).expect("terminal row remains");
    assert_eq!(still_ready.state, READY, "later packets cannot regress terminal state");
    assert_eq!(still_ready.meta, expected, "later packets cannot replace terminal metadata");

    child.remove(&child_cookie).expect("one-time userspace take deletes row");
    send_burst(client.as_raw_fd(), 32);
    thread::sleep(Duration::from_millis(50));
    wait_missing(child, child_cookie);
    drop(accepted);
    drop(client);
    gate.remove();
    println!("terminal_delete_monotonic child_cookie={child_cookie}");
}

fn pending_release_is_final(
    origin: &mut AyaHashMap<aya::maps::MapData, u64, Meta>,
    child: &AyaHashMap<aya::maps::MapData, u64, ChildState>,
    listener: RawFd,
    target: SocketAddrV4,
) {
    let gate = FinalAckGate::install();
    let client = socket().expect("release client");
    FinalAckGate::select(client.as_raw_fd());
    let app_cookie = cookie(client.as_raw_fd()).expect("release app cookie");
    origin.insert(app_cookie, meta_for(app_cookie, 22), 0).expect("release origin");
    assert_eq!(
        fast_open_send(client.as_raw_fd(), target, b"release-syn").expect("release MSG_FASTOPEN"),
        11
    );
    let accepted = accept(listener).expect("accept release child");
    let child_cookie = cookie(accepted.as_raw_fd()).expect("release child cookie");
    assert!(
        child.get(&child_cookie, 0).is_ok(),
        "Fast Open passive callback creates the pre-terminal row"
    );
    let sender = duplicate(client.as_raw_fd());
    let burst = thread::spawn(move || send_burst(sender.as_raw_fd(), 128));
    set_reset_close(accepted.as_raw_fd());
    drop(accepted);
    wait_missing(child, child_cookie);
    gate.remove();
    burst.join().expect("release sender");
    thread::sleep(Duration::from_millis(100));
    wait_missing(child, child_cookie);
    drop(client);
    println!("pending_release_final child_cookie={child_cookie}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: probe <bpf-object> <cgroup-dir>");
    ensure_cgroup(&args[2]);
    let intake_cgroup = format!("{}/intake", args[2]);
    fs::create_dir(&intake_cgroup).expect("create private intake cgroup");
    ensure_bpffs();
    println!(
        "uname={}",
        fs::read_to_string("/proc/sys/kernel/osrelease").expect("read kernel release").trim()
    );

    let pin_dir = format!("/sys/fs/bpf/tcp-correlation-ingress-{}", std::process::id());
    let active_pin = format!("{pin_dir}/ACTIVE");
    let child_pin = format!("{pin_dir}/CHILD");
    fs::create_dir(&pin_dir).expect("create private transient pin directory");
    let correlation_btf = load_correlation_btf().expect("load correlation BTF");
    let active_storage =
        create_active_storage(correlation_btf.as_raw_fd()).expect("create active socket storage");
    let child_storage =
        create_child_map(correlation_btf.as_raw_fd()).expect("create locked child map");
    pin_object(active_storage.as_raw_fd(), &active_pin).expect("pin active storage for Aya reuse");
    pin_object(child_storage.as_raw_fd(), &child_pin).expect("pin child map for Aya reuse");
    let mut loader = EbpfLoader::new();
    loader.allow_unsupported_maps().map_pin_path(&pin_dir);
    let mut ebpf = loader.load_file(&args[1]).expect("load Aya object");
    fs::remove_file(&active_pin).expect("remove active pin before program attach");
    fs::remove_file(&child_pin).expect("remove child pin before program attach");
    fs::remove_dir(&pin_dir).expect("remove transient pin directory before program attach");
    drop((active_storage, child_storage, correlation_btf));
    {
        let program: &mut SocketFilter =
            ebpf.program_mut("inherited_socket_filter_lock_probe").unwrap().try_into().unwrap();
        let error = program.load().expect_err("locked child map must reject socket-filter use");
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("socket filter progs cannot use bpf_spin_lock yet"),
            "unexpected socket-filter verifier result: {diagnostic}"
        );
        println!("socket_filter_locked_map=REJECTED verifier={diagnostic}");
    }
    {
        let program: &mut SockOps =
            ebpf.program_mut("tcp_correlation").unwrap().try_into().unwrap();
        program.load().expect("load SockOps program");
        program
            .attach(File::open(&args[2]).expect("open cgroup"), CgroupAttachMode::Single)
            .expect("attach SockOps program");
    }
    {
        let program: &mut CgroupSkb =
            ebpf.program_mut("tcp_correlation_ingress").unwrap().try_into().unwrap();
        program.load().expect("load cgroup ingress program");
        program
            .attach(
                File::open(&intake_cgroup).expect("open intake cgroup"),
                CgroupSkbAttachType::Ingress,
                CgroupAttachMode::Single,
            )
            .expect("attach cgroup ingress program");
    }
    {
        let program: &mut CgroupSock =
            ebpf.program_mut("correlation_release").unwrap().try_into().unwrap();
        program.load().expect("load release program");
        program
            .attach(File::open(&args[2]).expect("open cgroup"), CgroupAttachMode::Single)
            .expect("attach release program");
    }
    join_cgroup(&args[2]);

    let mut origin: AyaHashMap<_, u64, Meta> =
        AyaHashMap::try_from(ebpf.take_map("ORIGIN").unwrap()).unwrap();
    let mut child: AyaHashMap<_, u64, ChildState> =
        AyaHashMap::try_from(ebpf.take_map("CHILD").unwrap()).unwrap();
    let counter_map: PerCpuArray<_, u64> =
        PerCpuArray::try_from(ebpf.take_map("COUNTERS").unwrap()).unwrap();

    root_socket_bypasses_private_ingress(&mut child, &counter_map);

    let normal_listener =
        listener_in_cgroup(&args[2], &intake_cgroup, false).expect("normal listener");
    let normal_target = local(normal_listener.as_raw_fd()).unwrap();
    let ordinary =
        captured_connect(&mut origin, &child, normal_listener.as_raw_fd(), normal_target, 9);
    println!(
        "ordinary_exact app_cookie={} child_cookie={} meta={:?}",
        ordinary.0, ordinary.1, ordinary.2
    );

    let direct = socket().unwrap();
    connect(direct.as_raw_fd(), normal_target).expect("direct connect");
    let direct_child = accept(normal_listener.as_raw_fd()).expect("accept direct child");
    let direct_cookie = cookie(direct_child.as_raw_fd()).unwrap();
    let direct_state = wait_child(&child, direct_cookie, REJECT);
    assert_eq!(direct_state.state, REJECT);
    println!("direct_rejected child_cookie={direct_cookie}");

    let syncookie_path = "/proc/sys/net/ipv4/tcp_syncookies";
    let original_syncookies = fs::read_to_string(syncookie_path).unwrap();
    write_sysctl(syncookie_path, "2");
    let syncookies_before = tcpext("SyncookiesRecv");
    let sync_listener =
        listener_in_cgroup(&args[2], &intake_cgroup, false).expect("syncookie listener");
    let sync_target = local(sync_listener.as_raw_fd()).unwrap();
    let sync = captured_connect(&mut origin, &child, sync_listener.as_raw_fd(), sync_target, 10);
    let syncookies_after = tcpext("SyncookiesRecv");
    assert!(syncookies_after > syncookies_before, "forced syncookie path executed");
    write_sysctl(syncookie_path, original_syncookies.trim());
    println!(
        "syncookie_exact delta={} app_cookie={} child_cookie={} meta={:?}",
        syncookies_after - syncookies_before,
        sync.0,
        sync.1,
        sync.2
    );

    let fastopen_path = "/proc/sys/net/ipv4/tcp_fastopen";
    let original_fastopen = fs::read_to_string(fastopen_path).unwrap();
    write_sysctl(fastopen_path, "3");
    let tfo_listener =
        listener_in_cgroup(&args[2], &intake_cgroup, true).expect("Fast Open listener");
    let tfo_target = local(tfo_listener.as_raw_fd()).unwrap();
    let _prime =
        captured_fast_open(&mut origin, &child, tfo_listener.as_raw_fd(), tfo_target, 11, b"prime");
    let _direct_tfo =
        direct_fast_open_rejected(&child, tfo_listener.as_raw_fd(), tfo_target, b"direct-tfo");
    let tfo_before = tcpext("TCPFastOpenPassive");
    let tfo = captured_fast_open(
        &mut origin,
        &child,
        tfo_listener.as_raw_fd(),
        tfo_target,
        12,
        b"tfo-data",
    );
    let tfo_after = tcpext("TCPFastOpenPassive");
    assert!(tfo_after > tfo_before, "data-in-SYN Fast Open path executed");
    terminal_and_delete_are_monotonic(
        &mut origin,
        &mut child,
        tfo_listener.as_raw_fd(),
        tfo_target,
    );
    pending_release_is_final(&mut origin, &child, tfo_listener.as_raw_fd(), tfo_target);
    write_sysctl(fastopen_path, original_fastopen.trim());
    println!(
        "fastopen_exact delta={} app_cookie={} child_cookie={} meta={:?}",
        tfo_after - tfo_before,
        tfo.0,
        tfo.1,
        tfo.2
    );

    let unused = listener(false).expect("reserve unused port");
    let unused_target = local(unused.as_raw_fd()).unwrap();
    drop(unused);
    let refused = socket().unwrap();
    let refused_cookie = cookie(refused.as_raw_fd()).unwrap();
    origin.insert(refused_cookie, meta_for(refused_cookie, 13), 0).unwrap();
    let refused_error =
        connect(refused.as_raw_fd(), unused_target).expect_err("unused port must refuse");
    assert_eq!(refused_error.kind(), io::ErrorKind::ConnectionRefused);
    println!("refused_cleanup app_cookie={refused_cookie}");

    let counts = read_counters(&counter_map);
    println!("counters={counts:?}");
    assert!(counts[0] >= 4, "captured sockets armed");
    assert!(counts[1] >= 4, "active-established emission armed");
    assert!(counts[2] >= 4 && counts[3] >= 4, "ACK option reserved and written");
    assert!(counts[4] >= 2, "ordinary/syncookie passive ACK decoded");
    assert!(counts[5] >= 2 && counts[6] >= 1, "Fast Open Pending -> Ready exercised");
    assert!(counts[7] >= 2, "ordinary and Fast Open direct missing options rejected");
    assert!(counts[12] >= 2, "ingress recorded captured final-ACK candidates");
    assert!(counts[13] >= 1, "ingress recorded the direct final-ACK rejection candidate");
    assert!(counts[14] >= 1, "later data observed a terminal row and was a no-op");
    assert_eq!(counts[8], 0, "no store error");
    assert!(counts[10] >= 4, "a post-handshake response deleted active storage");
    assert!(counts[11] >= 1, "TCP_CLOSE observed unconfirmed active storage before destruction");
    println!("VERDICT=WORKS");
}
