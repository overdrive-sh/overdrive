use std::convert::TryInto as _;
use std::ffi::CString;
use std::fs;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::os::fd::FromRawFd as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aya::maps::{Array, HashMap, Map, MapData, MapInfo};
use aya::programs::tc::{SchedClassifierLink, TcAttachOptions};
use aya::programs::{LinkOrder, SchedClassifier, TcAttachType, links::{FdLink, PinnedLink}};
use aya::{Ebpf, Pod};

const ENDPOINT_MAP: &str = "endpoints";
const COUNTERS_MAP: &str = "counters";
const LINK_A: &str = "link-tap295ca";
const LINK_B: &str = "link-tap295cb";
const COUNTER_NAMES: [&str; 8] = [
    "gateway_pass",
    "intercept",
    "map_miss",
    "spoof_mac",
    "spoof_ip",
    "direct_bypass_drop",
    "arp_pass",
    "malformed_drop",
];

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Endpoint {
    source_ip: u32,
    source_mac: [u8; 6],
    source_pad: [u8; 2],
    bridge_mac: [u8; 6],
    bridge_pad: [u8; 2],
}

unsafe impl Pod for Endpoint {}

fn pin(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

fn ifindex(name: &str) -> u32 {
    let name = CString::new(name).unwrap();
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert!(index != 0, "if_nametoindex failed: {}", std::io::Error::last_os_error());
    index
}

fn parse_mac(raw: &str) -> [u8; 6] {
    let octets = raw
        .split(':')
        .map(|part| u8::from_str_radix(part, 16).unwrap())
        .collect::<Vec<_>>();
    octets.try_into().unwrap()
}

fn endpoint(ip: &str, mac: &str) -> Endpoint {
    Endpoint {
        source_ip: u32::from(ip.parse::<Ipv4Addr>().unwrap()),
        source_mac: parse_mac(mac),
        source_pad: [0; 2],
        bridge_mac: parse_mac("02:00:00:95:00:01"),
        bridge_pad: [0; 2],
    }
}

fn load(object: &Path, root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    let load_start = Instant::now();
    let mut bpf = Ebpf::load_file(object)?;
    bpf.map("ENDPOINTS").unwrap().pin(pin(root, ENDPOINT_MAP))?;
    bpf.map("COUNTERS").unwrap().pin(pin(root, COUNTERS_MAP))?;
    let program: &mut SchedClassifier = bpf.program_mut("gh295c_endpoint").unwrap().try_into()?;
    program.load()?;
    let info = program.info()?;
    println!(
        "TCX_PROGRAM_LOADED id={} verified_insns={:?} memlock_bytes={} elapsed_ms={:.3}",
        info.id(),
        info.verified_instruction_count(),
        info.memory_locked()?,
        load_start.elapsed().as_secs_f64() * 1000.0
    );
    for (iface, link_name) in [("tap295ca", LINK_A), ("tap295cb", LINK_B)] {
        let start = Instant::now();
        let id = program.attach_with_options(
            iface,
            TcAttachType::Ingress,
            TcAttachOptions::TcxOrder(LinkOrder::first()),
        )?;
        let link: SchedClassifierLink = program.take_link(id)?;
        let fd_link: FdLink = link.try_into()?;
        fd_link.pin(pin(root, link_name))?;
        println!(
            "TCX_LINK_PINNED iface={iface} ifindex={} pin={} attach_ms={:.3}",
            ifindex(iface),
            pin(root, link_name).display(),
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    println!("TCX_LOADER_EXITING links_and_maps_pinned=true");
    Ok(())
}

fn register(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = MapData::from_pin(pin(root, ENDPOINT_MAP))?;
    let mut endpoints = HashMap::<MapData, u32, Endpoint>::try_from(Map::HashMap(data))?;
    endpoints.insert(ifindex("tap295ca"), endpoint("10.95.0.2", "02:00:00:95:00:02"), 0)?;
    endpoints.insert(ifindex("tap295cb"), endpoint("10.95.0.3", "02:00:00:95:00:03"), 0)?;
    println!("TCX_ENDPOINTS_REGISTERED count=2");
    Ok(())
}

fn counters(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = MapData::from_pin(pin(root, COUNTERS_MAP))?;
    let counters = Array::<MapData, u64>::try_from(Map::Array(data))?;
    for (index, name) in COUNTER_NAMES.iter().enumerate() {
        println!("TCX_COUNTER {name}={}", counters.get(&(index as u32), 0)?);
    }
    Ok(())
}

fn inspect(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for link_name in [LINK_A, LINK_B] {
        let _adopted = PinnedLink::from_pin(pin(root, link_name))?;
        println!("TCX_LINK_ADOPTED pin={} live=true", pin(root, link_name).display());
    }
    for iface in ["tap295ca", "tap295cb"] {
        let (revision, programs) = SchedClassifier::query_tcx(iface, TcAttachType::Ingress)?;
        println!("TCX_QUERY iface={iface} revision={revision} programs={}", programs.len());
        for info in programs {
            println!(
                "TCX_QUERY_PROGRAM iface={iface} id={} name={} verified_insns={:?} memlock_bytes={}",
                info.id(),
                String::from_utf8_lossy(info.name()),
                info.verified_instruction_count(),
                info.memory_locked()?
            );
        }
    }
    for map_name in [ENDPOINT_MAP, COUNTERS_MAP] {
        let info = MapInfo::from_pin(pin(root, map_name))?;
        println!(
            "TCX_MAP pin={} id={} type={:?} key_size={} value_size={} max_entries={}",
            pin(root, map_name).display(),
            info.id(),
            info.map_type()?,
            info.key_size(),
            info.value_size(),
            info.max_entries()
        );
    }
    counters(root)
}

fn checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in header.chunks(2) {
        let word = if chunk.len() == 2 { u16::from_be_bytes([chunk[0], chunk[1]]) } else { u16::from(chunk[0]) << 8 };
        sum += u32::from(word);
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn frame(kind: &str) -> Vec<u8> {
    let (source_mac, source_ip, protocol) = match kind {
        "map-miss" => (parse_mac("02:00:00:95:00:02"), Ipv4Addr::new(10, 95, 0, 2), 6),
        "spoof-mac" => (parse_mac("02:00:00:95:00:99"), Ipv4Addr::new(10, 95, 0, 2), 6),
        "spoof-ip" => (parse_mac("02:00:00:95:00:02"), Ipv4Addr::new(10, 95, 0, 99), 6),
        "direct-bypass" => (parse_mac("02:00:00:95:00:02"), Ipv4Addr::new(10, 95, 0, 2), 17),
        other => panic!("unknown injection kind {other}"),
    };
    let mut bytes = vec![0u8; 14 + 20 + if protocol == 6 { 20 } else { 8 }];
    bytes[0..6].copy_from_slice(&parse_mac("02:00:00:95:00:03"));
    bytes[6..12].copy_from_slice(&source_mac);
    bytes[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    bytes[14] = 0x45;
    let ip_length = (bytes.len() - 14) as u16;
    bytes[16..18].copy_from_slice(&ip_length.to_be_bytes());
    bytes[22] = 64;
    bytes[23] = protocol;
    bytes[26..30].copy_from_slice(&source_ip.octets());
    bytes[30..34].copy_from_slice(&Ipv4Addr::new(10, 95, 0, 3).octets());
    let ip_checksum = checksum(&bytes[14..34]);
    bytes[24..26].copy_from_slice(&ip_checksum.to_be_bytes());
    bytes[34..36].copy_from_slice(&41000u16.to_be_bytes());
    bytes[36..38].copy_from_slice(&(if protocol == 6 { 9000u16 } else { 9999u16 }).to_be_bytes());
    if protocol == 6 {
        bytes[46] = 0x50;
        bytes[47] = 0x02;
        bytes[48..50].copy_from_slice(&64240u16.to_be_bytes());
    } else {
        bytes[38..40].copy_from_slice(&8u16.to_be_bytes());
    }
    bytes
}

fn inject(iface: &str, kind: &str) -> Result<(), Box<dyn std::error::Error>> {
    const TUNSETIFF: libc::c_ulong = 0x400454ca;
    const IFF_TAP: i16 = 0x0002;
    const IFF_NO_PI: i16 = 0x1000;
    let fd = unsafe { libc::open(c"/dev/net/tun".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (target, source) in request.ifr_name.iter_mut().zip(iface.bytes()) {
        *target = source as libc::c_char;
    }
    request.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;
    let rc = unsafe { libc::ioctl(fd, TUNSETIFF, &mut request) };
    if rc < 0 {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error().into());
    }
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    let bytes = frame(kind);
    file.write_all(&bytes)?;
    file.flush()?;
    println!("TCX_INJECT iface={iface} kind={kind} bytes={}", bytes.len());
    Ok(())
}

fn cleanup(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for link_name in [LINK_A, LINK_B] {
        let pinned = PinnedLink::from_pin(pin(root, link_name))?;
        let fd = pinned.unpin()?;
        drop(fd);
        println!("TCX_LINK_UNPINNED_AND_DETACHED pin={}", pin(root, link_name).display());
    }
    for map_name in [ENDPOINT_MAP, COUNTERS_MAP] {
        fs::remove_file(pin(root, map_name))?;
        println!("TCX_MAP_UNPINNED pin={}", pin(root, map_name).display());
    }
    fs::remove_dir(root)?;
    for iface in ["tap295ca", "tap295cb"] {
        let (_, programs) = SchedClassifier::query_tcx(iface, TcAttachType::Ingress)?;
        println!("TCX_CLEANUP_QUERY iface={iface} programs={}", programs.len());
        assert!(programs.is_empty());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let root = Path::new(args.get(2).map(String::as_str).unwrap_or("/sys/fs/bpf/gh295c"));
    match command {
        "load" => load(Path::new(args.get(3).unwrap()), root),
        "register" => register(root),
        "inspect" | "counters" => inspect(root),
        "inject" => inject(args.get(3).unwrap(), args.get(4).unwrap()),
        "cleanup" => cleanup(root),
        other => panic!("unknown command {other}"),
    }
}
