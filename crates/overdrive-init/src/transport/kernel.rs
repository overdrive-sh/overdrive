#![allow(
    unsafe_code,
    reason = "Linux netlink, BPF map staging and socket options are narrow syscall boundaries"
)]

use std::ffi::CString;
use std::fs::{self, File};
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use aya::maps::{Array, Queue};
use aya::programs::{CgroupAttachMode, CgroupSock, CgroupSockAddr, FEntry, SockOps};
use aya::{Btf, Ebpf, EbpfError, EbpfLoader};
use nix::errno::Errno;
use nix::kmod::{ModuleInitFlags, finit_module};
use nix::libc;
use nix::net::if_::if_nametoindex;
use nix::sys::socket::{
    AddressFamily, NetlinkAddr, SockFlag, SockProtocol, SockType, VsockAddr, bind, recv, sendto,
    socket,
};
use overdrive_core::guest_transport::forwarder::FlowForwarder;
use overdrive_core::guest_transport::{
    GUEST_FORWARDER_MODULE_PATH, GUEST_INBOUND_PORT, GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE,
    GUEST_UDP_SLOTS,
};
use overdrive_fwd::KernelFlowForwarder;

use super::{
    GUEST_TPROXY_MARK, GuestProgram, GuestTransportOps, GuestTransportSetupError,
    GuestTransportToken, KernelRelease,
};

const DUMMY_IFNAME: &str = "ovd0";
const CGROUP_PATH: &str = "/sys/fs/cgroup";
const CGROUP2_FS_MAGIC: libc::c_long = 0x6367_7270;
const BPF_FS_PATH: &str = "/sys/fs/bpf";
const BPF_FS_MAGIC: libc::c_long = 0xcafe_4a11;
const RULE_PRIORITY: u32 = 32_765;
const GUEST_ROUTE_TABLE: u32 = 295;
const INBOUND_VSOCK_BACKLOG: libc::c_int = 4_096;
#[repr(align(8))]
struct AlignedBpfObject<const N: usize>([u8; N]);

static BPF_OBJECT: AlignedBpfObject<
    { include_bytes!(env!("OVERDRIVE_GUEST_BPF_OBJECT_PATH")).len() },
> = AlignedBpfObject(*include_bytes!(env!("OVERDRIVE_GUEST_BPF_OBJECT_PATH")));
static TCP_CORRELATION_CANDIDATE_BPF_OBJECT: AlignedBpfObject<
    { include_bytes!(env!("OVERDRIVE_GUEST_TCP_CORRELATION_CANDIDATE_BPF_OBJECT_PATH")).len() },
> = AlignedBpfObject(*include_bytes!(env!(
    "OVERDRIVE_GUEST_TCP_CORRELATION_CANDIDATE_BPF_OBJECT_PATH"
)));
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) struct KernelGuestTransportOps {
    pub(crate) token: Option<GuestTransportToken>,
    pub(crate) cgroup: Option<File>,
    pub(crate) tcp_intake: Option<TcpListener>,
    pub(crate) udp_slots: Vec<UdpSocket>,
    pub(crate) inbound_listener: Option<OwnedFd>,
    pub(crate) bpf: Option<Ebpf>,
    candidate_bpf: Option<Ebpf>,
    pub(crate) forwarder: Option<KernelFlowForwarder>,
}

impl KernelGuestTransportOps {
    pub(crate) fn new() -> Self {
        let token = fs::read_to_string("/proc/cmdline")
            .ok()
            .and_then(|cmdline| super::parse_transport_token(&cmdline).ok());
        Self {
            token,
            cgroup: None,
            tcp_intake: None,
            udp_slots: Vec::with_capacity(usize::from(GUEST_UDP_SLOTS)),
            inbound_listener: None,
            bpf: None,
            candidate_bpf: None,
            forwarder: None,
        }
    }

    fn initialize_guest_maps(&mut self) -> Result<(), EbpfError> {
        let workload = self.token.map_or(Ipv4Addr::UNSPECIFIED, |token| token.workload_addr);
        let bpf = self.bpf.as_mut().ok_or_else(|| missing_bpf_item("embedded BPF object"))?;
        let cfg_map = bpf.map_mut("CFG").ok_or_else(|| missing_bpf_item("CFG"))?;
        let cfg = Array::<_, u32>::try_from(cfg_map)?;
        let mut cfg = cfg;
        cfg.set(0, u32::from_ne_bytes(workload.octets()), 0)?;
        cfg.set(1, u32::from(GUEST_TCP_INTAKE_PORT), 0)?;
        cfg.set(2, u32::from(GUEST_UDP_SLOT_BASE), 0)?;
        cfg.set(3, u32::from(GUEST_UDP_SLOTS), 0)?;
        let free_map = bpf.map_mut("FREE_SLOTS").ok_or_else(|| missing_bpf_item("FREE_SLOTS"))?;
        let mut free = Queue::<_, u32>::try_from(free_map)?;
        for slot in 0..u32::from(GUEST_UDP_SLOTS) {
            free.push(slot, 0)?;
        }
        Ok(())
    }

    fn cgroup_file(&mut self) -> io::Result<File> {
        if self.cgroup.is_none() {
            // GuestKit's PID 1 mounts proc, sysfs and devtmpfs but leaves
            // `/sys/fs/cgroup` as a directory. cgroup socket hooks require
            // an fd from a cgroup2 mount.
            ensure_filesystem_mounted(CGROUP_PATH, "none", "cgroup2", CGROUP2_FS_MAGIC)?;
            self.cgroup = Some(File::open(CGROUP_PATH)?);
        }
        self.cgroup
            .as_ref()
            .ok_or_else(|| io::Error::other("cgroup descriptor was not opened"))?
            .try_clone()
    }

    fn load_aya_program(&mut self, program: GuestProgram) -> Result<(), EbpfError> {
        if program == GuestProgram::TcpCorrelationCandidate {
            let bpf = self
                .candidate_bpf
                .as_mut()
                .ok_or_else(|| aya::programs::ProgramError::NotLoaded)?;
            let prog: &mut FEntry = bpf
                .program_mut("tcp_correlation_candidate")
                .ok_or_else(|| missing_bpf_item("tcp_correlation_candidate"))?
                .try_into()?;
            let btf = Btf::from_sys_fs()?;
            prog.load("tcp_check_req", &btf)?;
            return Ok(());
        }

        let bpf = self.bpf.as_mut().ok_or(aya::programs::ProgramError::NotLoaded)?;
        let name = match program {
            GuestProgram::TcpCorrelation => "tcp_correlation",
            GuestProgram::TcpCorrelationCandidate => {
                return Err(aya::programs::ProgramError::NotLoaded.into());
            }
            GuestProgram::Connect4 => "connect4",
            GuestProgram::Sendmsg4 => "sendmsg4",
            GuestProgram::Getpeername4 => "getpeername4",
            GuestProgram::Recvmsg4 => "recvmsg4",
            GuestProgram::SockRelease => "sock_release",
        };
        match program {
            GuestProgram::TcpCorrelation => {
                let prog: &mut SockOps =
                    bpf.program_mut(name).ok_or_else(|| missing_bpf_item(name))?.try_into()?;
                prog.load()?;
            }
            GuestProgram::SockRelease => {
                let prog: &mut CgroupSock =
                    bpf.program_mut(name).ok_or_else(|| missing_bpf_item(name))?.try_into()?;
                prog.load()?;
            }
            _ => {
                let prog: &mut CgroupSockAddr =
                    bpf.program_mut(name).ok_or_else(|| missing_bpf_item(name))?.try_into()?;
                prog.load()?;
            }
        }
        Ok(())
    }
}

impl GuestTransportOps for KernelGuestTransportOps {
    fn prepare_guest_programs(&mut self) -> Result<(), GuestTransportSetupError> {
        let (rust, candidate) =
            load_guest_bpf(&BPF_OBJECT.0, &TCP_CORRELATION_CANDIDATE_BPF_OBJECT.0)?;
        self.bpf = Some(rust);
        self.candidate_bpf = Some(candidate);
        self.initialize_guest_maps().map_err(|source| GuestTransportSetupError::ProgramLoad {
            program: GuestProgram::TcpCorrelation,
            source,
        })
    }

    fn create_dummy_device(&mut self) -> io::Result<()> {
        bring_loopback_up()?;
        add_dummy_link(DUMMY_IFNAME)?;
        let index = if_nametoindex(DUMMY_IFNAME).map_err(errno_to_io)?;
        set_link_up(index)
    }

    fn add_workload_address(&mut self, addr: Ipv4Addr) -> io::Result<()> {
        let index = if_nametoindex(DUMMY_IFNAME).map_err(errno_to_io)?;
        add_ipv4_address(index, addr)
    }

    fn add_fwmark_rule(&mut self) -> io::Result<()> {
        add_guest_fwmark_rule()
    }

    fn add_fwmark_route(&mut self) -> io::Result<()> {
        add_guest_local_default_route()
    }

    fn enable_tcp_fwmark_accept(&mut self) -> io::Result<()> {
        fs::write("/proc/sys/net/ipv4/tcp_fwmark_accept", "1")
    }

    fn enable_fwmark_reflect(&mut self) -> io::Result<()> {
        fs::write("/proc/sys/net/ipv4/fwmark_reflect", "1")
    }

    fn reserve_ports(&mut self, ports: std::ops::RangeInclusive<u16>) -> io::Result<()> {
        fs::write(
            "/proc/sys/net/ipv4/ip_local_reserved_ports",
            format!("{}-{}", ports.start(), ports.end()),
        )
    }

    fn bind_tcp_intake(&mut self, addr: SocketAddrV4) -> io::Result<()> {
        let fd = cvt(unsafe {
            libc::socket(
                libc::AF_INET,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::IPPROTO_TCP,
            )
        })?;
        // SAFETY: `fd` is a fresh socket descriptor owned below.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        bind_inet4(fd.as_raw_fd(), addr)?;
        let backlog = 4_096_i32;
        set_socket_option(fd.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_FASTOPEN, &backlog)?;
        cvt(unsafe { libc::listen(fd.as_raw_fd(), backlog) })?;
        self.tcp_intake = Some(TcpListener::from(fd));
        Ok(())
    }

    fn bind_udp_slot(&mut self, addr: SocketAddrV4) -> io::Result<()> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        self.udp_slots.push(socket);
        Ok(())
    }

    fn load_program(&mut self, program: GuestProgram) -> Result<(), EbpfError> {
        self.load_aya_program(program)
    }

    fn attach_program(&mut self, program: GuestProgram) -> Result<(), aya::programs::ProgramError> {
        if program == GuestProgram::TcpCorrelationCandidate {
            let bpf = self.candidate_bpf.as_mut().ok_or(aya::programs::ProgramError::NotLoaded)?;
            let prog: &mut FEntry = bpf
                .program_mut("tcp_correlation_candidate")
                .ok_or_else(|| invalid_program_name("tcp_correlation_candidate"))?
                .try_into()?;
            prog.attach()?;
            return Ok(());
        }

        let cgroup = self.cgroup_file().map_err(|error| {
            aya::programs::ProgramError::SyscallError(aya::sys::SyscallError {
                call: "prepare cgroup v2 root",
                io_error: error,
            })
        })?;
        let bpf = self.bpf.as_mut().ok_or(aya::programs::ProgramError::NotLoaded)?;
        let name = match program {
            GuestProgram::TcpCorrelation => "tcp_correlation",
            GuestProgram::TcpCorrelationCandidate => {
                return Err(aya::programs::ProgramError::NotLoaded);
            }
            GuestProgram::Connect4 => "connect4",
            GuestProgram::Sendmsg4 => "sendmsg4",
            GuestProgram::Getpeername4 => "getpeername4",
            GuestProgram::Recvmsg4 => "recvmsg4",
            GuestProgram::SockRelease => "sock_release",
        };
        match program {
            GuestProgram::TcpCorrelation => {
                let prog: &mut SockOps =
                    bpf.program_mut(name).ok_or_else(|| invalid_program_name(name))?.try_into()?;
                prog.attach(cgroup, CgroupAttachMode::Single)?;
            }
            GuestProgram::SockRelease => {
                let prog: &mut CgroupSock =
                    bpf.program_mut(name).ok_or_else(|| invalid_program_name(name))?.try_into()?;
                prog.attach(cgroup, CgroupAttachMode::Single)?;
            }
            _ => {
                let prog: &mut CgroupSockAddr =
                    bpf.program_mut(name).ok_or_else(|| invalid_program_name(name))?.try_into()?;
                prog.attach(cgroup, CgroupAttachMode::Single)?;
            }
        }
        Ok(())
    }

    fn load_forwarder_module(&mut self) -> Result<(), GuestTransportSetupError> {
        let path = PathBuf::from(GUEST_FORWARDER_MODULE_PATH);
        let module_bytes = fs::read(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                GuestTransportSetupError::ForwarderModuleMissing { path: path.clone() }
            } else {
                GuestTransportSetupError::ForwarderModuleLoad { source: error }
            }
        })?;
        let module_release = module_vermagic_release(&module_bytes)
            .map_err(|source| GuestTransportSetupError::ForwarderModuleLoad { source })?;
        let kernel_release_text = fs::read_to_string("/proc/sys/kernel/osrelease")
            .map_err(|source| GuestTransportSetupError::ForwarderModuleLoad { source })?;
        let kernel_release =
            KernelRelease::from_str(kernel_release_text.trim()).map_err(|error| {
                GuestTransportSetupError::ForwarderModuleLoad {
                    source: io::Error::new(io::ErrorKind::InvalidData, error),
                }
            })?;
        if module_release != kernel_release {
            return Err(GuestTransportSetupError::ForwarderModuleRelease {
                module: module_release,
                kernel: kernel_release,
            });
        }
        let file = File::open(path)
            .map_err(|source| GuestTransportSetupError::ForwarderModuleLoad { source })?;
        let module_args =
            CString::new("").map_err(|error| GuestTransportSetupError::ForwarderModuleLoad {
                source: io::Error::new(io::ErrorKind::InvalidInput, error),
            })?;
        match finit_module(&file, &module_args, ModuleInitFlags::empty()) {
            Ok(()) | Err(Errno::EEXIST) => Ok(()),
            Err(error) => Err(GuestTransportSetupError::ForwarderModuleLoad {
                source: io::Error::from_raw_os_error(error as i32),
            }),
        }
    }

    fn open_forwarder(
        &mut self,
    ) -> Result<(), overdrive_core::guest_transport::forwarder::ForwarderError> {
        let forwarder = KernelFlowForwarder::open()?;
        forwarder.probe()?;
        self.forwarder = Some(forwarder);
        Ok(())
    }

    fn bind_inbound_vsock(&mut self) -> io::Result<()> {
        let socket = socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .map_err(errno_to_io)?;
        let addr = VsockAddr::new(libc::VMADDR_CID_ANY, GUEST_INBOUND_PORT.as_u32());
        bind(socket.as_raw_fd(), &addr).map_err(errno_to_io)?;
        // `nix::Backlog::new` validates against the target libc's
        // `SOMAXCONN`; musl defines that as 128 even though Linux accepts a
        // 4,096 request and clamps it to the running kernel's `somaxconn`.
        // Call listen(2) directly so the static-musl production binary
        // reaches that kernel clamp instead of rejecting its backlog in
        // userspace before the syscall.
        // SAFETY: `socket` owns a live AF_VSOCK stream fd and the backlog is
        // the positive, bounded listener request above.
        let listen_result = unsafe { libc::listen(socket.as_raw_fd(), INBOUND_VSOCK_BACKLOG) };
        if listen_result < 0 {
            return Err(io::Error::last_os_error());
        }
        self.inbound_listener = Some(socket);
        Ok(())
    }
}

fn load_guest_bpf(
    rust_bytes: &[u8],
    candidate_bytes: &[u8],
) -> Result<(Ebpf, Ebpf), GuestTransportSetupError> {
    ensure_filesystem_mounted(BPF_FS_PATH, "bpf", "bpf", BPF_FS_MAGIC).map_err(|error| {
        program_load_error(GuestProgram::TcpCorrelation, map_error("ACTIVE_CORRELATION", error))
    })?;
    let staging = staging_path();
    fs::create_dir(&staging).map_err(|error| {
        program_load_error(GuestProgram::TcpCorrelation, map_error("ACTIVE_CORRELATION", error))
    })?;
    let active_pin = staging.join("ACTIVE_CORRELATION");

    let rust_result = create_active_storage(&active_pin)
        .map_err(|error| map_error("ACTIVE_CORRELATION", error))
        .and_then(|()| {
            EbpfLoader::new().allow_unsupported_maps().map_pin_path(&staging).load(rust_bytes)
        });
    let rust = match rust_result {
        Ok(bpf) => bpf,
        Err(source) => {
            let _ = remove_staging(&staging);
            return Err(program_load_error(GuestProgram::TcpCorrelation, source));
        }
    };

    let candidate = match EbpfLoader::new()
        .allow_unsupported_maps()
        .map_pin_path(&staging)
        .load(candidate_bytes)
    {
        Ok(bpf) => bpf,
        Err(source) => {
            let _ = remove_staging(&staging);
            return Err(program_load_error(GuestProgram::TcpCorrelationCandidate, source));
        }
    };

    remove_staging(&staging).map_err(|error| {
        program_load_error(GuestProgram::TcpCorrelation, map_error("ACTIVE_CORRELATION", error))
    })?;
    Ok((rust, candidate))
}

fn staging_path() -> PathBuf {
    let seq = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Path::new(BPF_FS_PATH).join(format!("ovd-init-{}-{seq}", std::process::id()))
}

fn ensure_filesystem_mounted(
    mountpoint: &str,
    source: &str,
    filesystem: &str,
    magic: libc::c_long,
) -> io::Result<()> {
    fs::create_dir_all(mountpoint)?;
    let target = CString::new(mountpoint)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if filesystem_magic(&target)? == magic {
        return Ok(());
    }
    let source =
        CString::new(source).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let filesystem = CString::new(filesystem)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: the C strings remain live across mount(2); no filesystem data is supplied.
    if unsafe {
        libc::mount(source.as_ptr(), target.as_ptr(), filesystem.as_ptr(), 0, std::ptr::null())
    } < 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EBUSY)
            && filesystem_magic(&target).is_ok_and(|found| found == magic)
        {
            return Ok(());
        }
        return Err(error);
    }
    if filesystem_magic(&target)? == magic {
        Ok(())
    } else {
        Err(io::Error::other(format!("{filesystem:?} did not mount at {mountpoint}")))
    }
}

fn filesystem_magic(path: &CString) -> io::Result<libc::c_long> {
    // SAFETY: `statfs` initializes its output structure on success.
    let mut info = unsafe { std::mem::zeroed::<libc::statfs>() };
    // SAFETY: the path is a live NUL-terminated string and the output pointer is valid.
    cvt(unsafe { libc::statfs(path.as_ptr(), &raw mut info) })?;
    Ok(info.f_type as libc::c_long)
}

fn create_active_storage(pin: &Path) -> io::Result<()> {
    let btf = load_active_btf()?;
    let map = create_active_map(btf.as_raw_fd())?;
    pin_map(map.as_raw_fd(), pin)
}

fn load_active_btf() -> io::Result<OwnedFd> {
    let strings = b"\0int\0active\0";
    let mut types = Vec::new();
    types.extend_from_slice(&1_u32.to_ne_bytes());
    types.extend_from_slice(&(1_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&4_u32.to_ne_bytes());
    types.extend_from_slice(&32_u32.to_ne_bytes());
    types.extend_from_slice(&5_u32.to_ne_bytes());
    types.extend_from_slice(&(4_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&24_u32.to_ne_bytes());
    let mut btf = Vec::new();
    btf.extend_from_slice(&0xeb9f_u16.to_ne_bytes());
    btf.push(1);
    btf.push(0);
    btf.extend_from_slice(&24_u32.to_ne_bytes());
    btf.extend_from_slice(&0_u32.to_ne_bytes());
    let type_length = u32::try_from(types.len()).map_err(invalid_btf_length)?;
    let string_length = u32::try_from(strings.len()).map_err(invalid_btf_length)?;
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&string_length.to_ne_bytes());
    btf.extend_from_slice(&types);
    btf.extend_from_slice(strings);
    let mut attr = [0_u8; 128];
    put64(&mut attr, 0, btf.as_ptr() as u64);
    put32(&mut attr, 16, u32::try_from(btf.len()).map_err(invalid_btf_length)?);
    bpf_syscall(18, &mut attr).and_then(owned_fd)
}

fn create_active_map(btf_fd: i32) -> io::Result<OwnedFd> {
    let mut attr = [0_u8; 128];
    put32(&mut attr, 0, 24);
    put32(&mut attr, 4, 4);
    put32(&mut attr, 8, 24);
    put32(&mut attr, 12, 0);
    put32(&mut attr, 16, 1);
    attr[28..43].copy_from_slice(b"ACTIVE_CORREL\0\0");
    put32(&mut attr, 48, btf_fd.cast_unsigned());
    put32(&mut attr, 52, 1);
    put32(&mut attr, 56, 2);
    bpf_syscall(0, &mut attr).and_then(owned_fd)
}

fn pin_map(fd: i32, path: &Path) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bpffs path contains NUL"))?;
    let mut attr = [0_u8; 128];
    put64(&mut attr, 0, path.as_ptr() as u64);
    put32(&mut attr, 8, fd.cast_unsigned());
    bpf_syscall(6, &mut attr).map(|_| ())
}

fn remove_staging(staging: &Path) -> io::Result<()> {
    let mut first_error = None;
    for name in ["ACTIVE_CORRELATION", "CHILD_CORRELATION"] {
        match fs::remove_file(staging.join(name)) {
            Ok(()) => {}
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    match fs::remove_dir(staging) {
        Ok(()) => {}
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn map_error(name: &str, error: io::Error) -> EbpfError {
    let code = i64::from(error.raw_os_error().unwrap_or(libc::EIO));
    aya::maps::MapError::CreateError { name: name.to_owned(), code, io_error: error }.into()
}

const fn program_load_error(program: GuestProgram, source: EbpfError) -> GuestTransportSetupError {
    GuestTransportSetupError::ProgramLoad { program, source }
}

fn missing_bpf_item(name: &str) -> EbpfError {
    EbpfError::FileError {
        path: PathBuf::from(name),
        error: io::Error::new(io::ErrorKind::NotFound, format!("missing embedded BPF item {name}")),
    }
}

fn invalid_program_name(name: &str) -> aya::programs::ProgramError {
    aya::programs::ProgramError::InvalidName { name: name.to_owned() }
}

fn invalid_btf_length(error: std::num::TryFromIntError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

fn owned_fd(raw: i64) -> io::Result<OwnedFd> {
    let raw =
        i32::try_from(raw).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // SAFETY: the successful BPF syscall returned a new non-negative file descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn bpf_syscall(command: libc::c_long, attr: &mut [u8; 128]) -> io::Result<i64> {
    // SAFETY: each call passes a pointer to a correctly sized kernel bpf_attr
    // prefix and the kernel copies all pointed-to input before returning.
    let result = unsafe { libc::syscall(libc::SYS_bpf, command, attr.as_mut_ptr(), attr.len()) };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result as i64) }
}

fn put32(attr: &mut [u8; 128], offset: usize, value: u32) {
    attr[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn put64(attr: &mut [u8; 128], offset: usize, value: u64) {
    attr[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}

fn module_vermagic_release(bytes: &[u8]) -> io::Result<KernelRelease> {
    let marker = b"vermagic=";
    let start = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "module has no vermagic"))?
        + marker.len();
    let rest = &bytes[start..];
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "vermagic is not terminated"))?;
    let text = std::str::from_utf8(&rest[..end])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let release = text
        .split_ascii_whitespace()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "vermagic is empty"))?;
    KernelRelease::from_str(release)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn add_dummy_link(name: &str) -> io::Result<()> {
    let mut info = vec![0_u8; 16];
    info[0] = u8::try_from(libc::AF_UNSPEC)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    append_attr(&mut info, 3, format!("{name}\0").as_bytes());
    let mut link_info = Vec::new();
    append_attr(&mut link_info, 1, b"dummy\0");
    append_attr(&mut info, 0x0012 | 0x8000, &link_info);
    netlink_request(16, 1 | 4 | 0x400 | 0x200, &info)
}

fn set_link_up(index: u32) -> io::Result<()> {
    let mut info = vec![0_u8; 16];
    let index =
        i32::try_from(index).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    info[4..8].copy_from_slice(&index.to_ne_bytes());
    info[8..12].copy_from_slice(&libc::IFF_UP.cast_unsigned().to_ne_bytes());
    info[12..16].copy_from_slice(&libc::IFF_UP.cast_unsigned().to_ne_bytes());
    netlink_request(16, 1 | 4, &info)
}

fn add_ipv4_address(index: u32, addr: Ipv4Addr) -> io::Result<()> {
    let mut message = vec![0_u8; 8];
    message[0] = u8::try_from(libc::AF_INET)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    message[1] = 32;
    message[4..8].copy_from_slice(&index.to_ne_bytes());
    append_attr(&mut message, 1, &addr.octets()); // IFA_ADDRESS
    append_attr(&mut message, 2, &addr.octets()); // IFA_LOCAL
    netlink_request(20, 1 | 4 | 0x400 | 0x200, &message)
}

fn add_guest_fwmark_rule() -> io::Result<()> {
    let mut rule = vec![0_u8; 12];
    rule[0] = u8::try_from(libc::AF_INET)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    rule[4] = 0; // RT_TABLE_UNSPEC; FRA_TABLE carries the full table id.
    rule[7] = 1; // FR_ACT_TO_TBL
    append_attr(&mut rule, 6, &RULE_PRIORITY.to_ne_bytes()); // FRA_PRIORITY
    append_attr(&mut rule, 10, &GUEST_TPROXY_MARK.to_ne_bytes()); // FRA_FWMARK
    append_attr(&mut rule, 16, &u32::MAX.to_ne_bytes()); // FRA_FWMASK
    append_attr(&mut rule, 15, &GUEST_ROUTE_TABLE.to_ne_bytes()); // FRA_TABLE
    netlink_request(32, 1 | 4 | 0x400 | 0x200, &rule)
}

fn add_guest_local_default_route() -> io::Result<()> {
    let loopback_index = if_nametoindex("lo").map_err(errno_to_io)?;
    let mut route = vec![0_u8; 12];
    route[0] = u8::try_from(libc::AF_INET)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    route[4] = 0; // RT_TABLE_UNSPEC; RTA_TABLE carries the full table id.
    route[5] = 3; // RTPROT_BOOT
    route[6] = 254; // RT_SCOPE_HOST
    route[7] = 2; // RTN_LOCAL
    append_attr(&mut route, 4, &loopback_index.to_ne_bytes()); // RTA_OIF
    append_attr(&mut route, 15, &GUEST_ROUTE_TABLE.to_ne_bytes()); // RTA_TABLE
    netlink_request(24, 1 | 4 | 0x400 | 0x200, &route)
}

fn append_attr(message: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    let length = u16::try_from(4 + payload.len()).unwrap_or(u16::MAX);
    let start = message.len();
    message.extend_from_slice(&length.to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    message.resize(start + ((usize::from(length) + 3) & !3), 0);
}

fn netlink_request(kind: u16, flags: u16, payload: &[u8]) -> io::Result<()> {
    let fd = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        Some(SockProtocol::NetlinkRoute),
    )
    .map_err(errno_to_io)?;
    let kernel = NetlinkAddr::new(0, 0);
    bind(fd.as_raw_fd(), &NetlinkAddr::new(0, 0)).map_err(errno_to_io)?;
    let mut request = vec![0_u8; 16];
    request.extend_from_slice(payload);
    let length = u32::try_from(request.len()).unwrap_or(u32::MAX);
    request[0..4].copy_from_slice(&length.to_ne_bytes());
    request[4..6].copy_from_slice(&kind.to_ne_bytes());
    request[6..8].copy_from_slice(&flags.to_ne_bytes());
    request[8..12].copy_from_slice(&1_u32.to_ne_bytes());
    sendto(fd.as_raw_fd(), &request, &kernel, nix::sys::socket::MsgFlags::empty())
        .map_err(errno_to_io)?;
    let mut response = [0_u8; 4_096];
    let count = recv(fd.as_raw_fd(), &mut response, nix::sys::socket::MsgFlags::empty())
        .map_err(errno_to_io)?;
    if count < 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short rtnetlink ACK"));
    }
    let response_type = u16::from_ne_bytes([response[4], response[5]]);
    if response_type != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rtnetlink reply is not NLMSG_ERROR",
        ));
    }
    let error = i32::from_ne_bytes([response[16], response[17], response[18], response[19]]);
    if error == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(-error)) }
}

fn bind_inet4(fd: i32, addr: SocketAddrV4) -> io::Result<()> {
    let address = libc::sockaddr_in {
        sin_family: u16::try_from(libc::AF_INET)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        sin_port: addr.port().to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(addr.ip().octets()) },
        sin_zero: [0; 8],
    };
    // SAFETY: `address` is a fully initialized sockaddr_in of the size passed.
    cvt(unsafe {
        libc::bind(
            fd,
            (&raw const address).cast(),
            libc::socklen_t::try_from(core::mem::size_of::<libc::sockaddr_in>())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        )
    })?;
    Ok(())
}

fn set_socket_option<T>(fd: i32, level: i32, name: i32, value: &T) -> io::Result<()> {
    // SAFETY: `value` remains live for the duration of setsockopt and its size is exact.
    cvt(unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            std::ptr::from_ref(value).cast(),
            libc::socklen_t::try_from(core::mem::size_of::<T>())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        )
    })?;
    Ok(())
}

fn bring_loopback_up() -> io::Result<()> {
    let index = if_nametoindex("lo").map_err(errno_to_io)?;
    let mut message = vec![0_u8; 16];
    let index =
        i32::try_from(index).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    message[4..8].copy_from_slice(&index.to_ne_bytes());
    message[8..12].copy_from_slice(&libc::IFF_UP.cast_unsigned().to_ne_bytes());
    message[12..16].copy_from_slice(&libc::IFF_UP.cast_unsigned().to_ne_bytes());
    netlink_request(16, 1 | 4, &message)
}

fn cvt(value: i32) -> io::Result<i32> {
    if value < 0 { Err(io::Error::last_os_error()) } else { Ok(value) }
}

fn errno_to_io(error: Errno) -> io::Error {
    io::Error::from_raw_os_error(error as i32)
}
