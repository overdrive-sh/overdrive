use std::fs::{self, File};
use std::io;
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::thread;
use std::time::{Duration, Instant};

use aya::maps::{HashMap as AyaHashMap, PerCpuArray};
use aya::programs::{CgroupAttachMode, SockOps};
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
    state: u32,
    pad: u32,
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

/// Minimal BTF: type 1 is the 32-bit socket-fd key and type 2 is the
/// 24-byte active correlation value.
fn active_btf_blob() -> Vec<u8> {
    let strings = b"\0int\0active\0";
    let mut types = Vec::new();
    types.extend_from_slice(&1u32.to_ne_bytes());
    types.extend_from_slice(&(1u32 << 24).to_ne_bytes()); // BTF_KIND_INT
    types.extend_from_slice(&4u32.to_ne_bytes());
    types.extend_from_slice(&32u32.to_ne_bytes());
    types.extend_from_slice(&5u32.to_ne_bytes());
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
    blob.extend_from_slice(strings);
    blob
}

fn load_active_btf() -> Result<OwnedFd, i32> {
    let blob = active_btf_blob();
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
    put32(&mut attr, 56, 2);
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
    (0..12)
        .map(|i| map.get(&i, 0).map(|values| values.iter().copied().sum()).unwrap_or(0))
        .collect()
}

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
    drop(client);
    let accepted = accept(listener).expect("accept Fast Open child");
    let child_cookie = cookie(accepted.as_raw_fd()).unwrap();
    let got = wait_child(child, child_cookie, READY);
    assert_eq!(got.meta, expected, "Fast Open final ACK carries exact metadata");
    let mut buf = vec![0u8; data.len()];
    let n = unsafe { libc::recv(accepted.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
    assert_eq!(n, data.len() as isize, "Fast Open data readable");
    assert_eq!(&buf, data, "Fast Open data exact");
    (app_cookie, child_cookie, got.meta)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: probe <bpf-object> <cgroup-dir>");
    println!(
        "uname={}",
        String::from_utf8_lossy(
            &std::process::Command::new("uname").arg("-r").output().unwrap().stdout
        )
        .trim()
    );

    let pin_dir = format!("/sys/fs/bpf/tcp-correlation-option-{}", std::process::id());
    let pin_path = format!("{pin_dir}/ACTIVE");
    fs::create_dir(&pin_dir).expect("create private transient pin directory");
    let active_btf = load_active_btf().expect("load active storage BTF");
    let active_storage =
        create_active_storage(active_btf.as_raw_fd()).expect("create active socket storage");
    pin_object(active_storage.as_raw_fd(), &pin_path).expect("pin active storage for Aya reuse");
    let mut loader = EbpfLoader::new();
    loader.allow_unsupported_maps().map_pin_path(&pin_dir);
    let mut ebpf = loader.load_file(&args[1]).expect("load Aya object");
    fs::remove_file(&pin_path).expect("remove transient map pin before program attach");
    fs::remove_dir(&pin_dir).expect("remove transient pin directory before program attach");
    drop((active_storage, active_btf));
    {
        let program: &mut SockOps =
            ebpf.program_mut("tcp_correlation").unwrap().try_into().unwrap();
        program.load().expect("load SockOps program");
        program
            .attach(File::open(&args[2]).expect("open cgroup"), CgroupAttachMode::Single)
            .expect("attach SockOps program");
    }
    fs::write(format!("{}/cgroup.procs", args[2]), std::process::id().to_string())
        .expect("join cgroup");

    let mut origin: AyaHashMap<_, u64, Meta> =
        AyaHashMap::try_from(ebpf.take_map("ORIGIN").unwrap()).unwrap();
    let child: AyaHashMap<_, u64, ChildState> =
        AyaHashMap::try_from(ebpf.take_map("CHILD").unwrap()).unwrap();
    let counter_map: PerCpuArray<_, u64> =
        PerCpuArray::try_from(ebpf.take_map("COUNTERS").unwrap()).unwrap();

    let normal_listener = listener(false).expect("normal listener");
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
    let sync_listener = listener(false).expect("syncookie listener");
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
    let tfo_listener = listener(true).expect("Fast Open listener");
    let tfo_target = local(tfo_listener.as_raw_fd()).unwrap();
    let _prime =
        captured_fast_open(&mut origin, &child, tfo_listener.as_raw_fd(), tfo_target, 11, b"prime");
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
    assert!(counts[5] >= 1 && counts[6] >= 1, "Fast Open Pending -> Ready exercised");
    assert!(counts[7] >= 1, "direct missing option rejected");
    assert_eq!(counts[8], 0, "no store error");
    assert!(counts[10] >= 4, "a post-handshake response deleted active storage");
    assert!(counts[11] >= 1, "TCP_CLOSE observed unconfirmed active storage before destruction");
    println!("VERDICT=WORKS");
}
