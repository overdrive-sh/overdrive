//! A stand-in for `cloud-hypervisor`, built inside a guest by a test and handed to
//! `CloudHypervisorVmm::with_binary`. Test fixture source, never production code.
//!
//! It answers the two probes the launcher makes (`--version`, `--help` naming `--landlock`).
//! Launched with the real hypervisor's argument list, its FIRST act is to record the cgroup it
//! was born into (`/proc/self/cgroup`) and its pid into `standin.record` in its run directory
//! (the parent of the `--api-socket` path, which the launcher chowns to the confined identity).
//! It then claims the guest CID the launch leased (`--vsock cid=<n>,backend=vhost-kernel`) the
//! way a hypervisor does: it opens `/dev/vhost-vsock`, takes ownership and sets the guest CID,
//! and appends the outcome to the record. It then blocks, holding the device, until killed.
//!
//! Built static-musl with `rustc` of the `nested-launch-rustc` guest; it uses only `std` and the C library
//! `ioctl` the standard library already links.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

/// `VHOST_SET_OWNER` = `_IO(0xAF, 0x01)`.
const VHOST_SET_OWNER: u64 = 0xAF01;
/// `VHOST_VSOCK_SET_GUEST_CID` = `_IOW(0xAF, 0x60, __u64)`.
const VHOST_VSOCK_SET_GUEST_CID: u64 = 0x4008_AF60;

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("cloud-hypervisor stand-in");
        return;
    }
    if args.iter().any(|a| a == "--help") {
        println!("--landlock");
        return;
    }

    let run_dir: PathBuf = value_after(&args, "--api-socket")
        .and_then(|socket| std::path::Path::new(socket).parent())
        .map(PathBuf::from)
        .expect("launched with --api-socket");
    let mut record = OpenOptions::new()
        .create(true)
        .append(true)
        .open(run_dir.join("standin.record"))
        .expect("open the record in the run directory");

    // First act: where was this process born?
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    for line in cgroup.lines() {
        writeln!(record, "cgroup={line}").expect("record the cgroup");
    }
    writeln!(record, "pid={}", std::process::id()).expect("record the pid");

    // Then hold the guest CID the way a hypervisor does.
    let cid: u64 = value_after(&args, "--vsock")
        .and_then(|arg| arg.split(',').find_map(|field| field.strip_prefix("cid=")))
        .and_then(|cid| cid.parse().ok())
        .expect("launched with --vsock cid=<n>");
    writeln!(record, "cid={cid}").expect("record the cid");
    let device = OpenOptions::new().read(true).write(true).open("/dev/vhost-vsock");
    let claim = match device.as_ref() {
        Ok(file) => {
            use std::os::fd::AsRawFd as _;
            let fd = file.as_raw_fd();
            // SAFETY: `fd` is a live descriptor of this process; both requests take no pointer
            // (`SET_OWNER`) or a pointer to one live `u64` (`SET_GUEST_CID`).
            let (owner, set) = unsafe {
                (ioctl(fd, VHOST_SET_OWNER, 0_u64), ioctl(fd, VHOST_VSOCK_SET_GUEST_CID, &cid))
            };
            if owner == 0 && set == 0 {
                "claim=ok".to_owned()
            } else {
                format!(
                    "claim=errno-{}",
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
                )
            }
        }
        Err(error) => format!("claim=open-{error}"),
    };
    writeln!(record, "{claim}").expect("record the claim");
    drop(record);

    // Hold the device (the descriptor lives in `device`) until killed.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
