//! v30 guest probe (static, runs inside the microVM; spawned by /init).
//!
//! Each boot of the guest prints GUEST_BOOT <boot_id>, then
//!   - listens on vsock port 5000: on accept, sends "INC <boot_id>\n" and keeps the connection,
//!   - connects to the host (CID 2) port 6000: sends "HELLO <boot_id>\n", then "PING n\n" every 100 ms,
//!     and reads "GO <case>\n" from the host: writes <case> to /tmp/go (the /init shell acts on it).
//! Everything is also printed to stdout (the serial console) with a monotonic stamp.
use std::io::{Read, Write};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string()
}

fn addr(cid: u32, port: u32) -> libc::sockaddr_vm {
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    a.svm_family = libc::AF_VSOCK as _;
    a.svm_cid = cid;
    a.svm_port = port;
    a
}

fn sock() -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn main() {
    let t0 = Instant::now();
    let bid = boot_id();
    println!("[{:>8.3}] GUEST_BOOT boot_id={bid}", t0.elapsed().as_secs_f64());

    // listener (host -> guest connections)
    let lbid = bid.clone();
    std::thread::spawn(move || {
        let l = loop {
            match sock() {
                Ok(fd) => {
                    let a = addr(libc::VMADDR_CID_ANY, 5000);
                    let r = unsafe { libc::bind(fd.as_raw_fd(), &a as *const _ as *const _, 16) };
                    let r2 = if r == 0 { unsafe { libc::listen(fd.as_raw_fd(), 16) } } else { r };
                    if r2 == 0 {
                        break fd;
                    }
                    println!("listen failed: {}", std::io::Error::last_os_error());
                }
                Err(e) => println!("socket failed: {e}"),
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        println!("GUEST_LISTEN_READY");
        loop {
            let c = unsafe { libc::accept4(l.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
            if c < 0 {
                println!("accept err {}", std::io::Error::last_os_error());
                continue;
            }
            let b = lbid.clone();
            std::thread::spawn(move || {
                let mut f = unsafe { std::fs::File::from_raw_fd(c as RawFd) };
                let _ = f.write_all(format!("INC {b}\n").as_bytes());
                let mut buf = [0u8; 256];
                loop {
                    match f.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            });
        }
    });

    // outbound connection to the host listener
    let s = loop {
        let fd = match sock() {
            Ok(f) => f,
            Err(e) => {
                println!("socket failed: {e}");
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let a = addr(2, 6000);
        let r = unsafe { libc::connect(fd.as_raw_fd(), &a as *const _ as *const _, 16) };
        if r == 0 {
            break fd;
        }
        println!("connect to host failed: {}", std::io::Error::last_os_error());
        std::thread::sleep(Duration::from_millis(200));
    };
    let raw = s.into_raw_fd();
    let mut w = unsafe { std::fs::File::from_raw_fd(raw) };
    let mut r = unsafe { std::fs::File::from_raw_fd(libc::dup(raw)) };
    let _ = w.write_all(format!("HELLO {bid}\n").as_bytes());
    println!("[{:>8.3}] GUEST_HELLO_SENT", t0.elapsed().as_secs_f64());
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            match r.read(&mut buf) {
                Ok(0) => {
                    println!("host conn EOF");
                    break;
                }
                Err(e) => {
                    println!("host conn read err {e}");
                    break;
                }
                Ok(n) => {
                    let t = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(c) = t.strip_prefix("GO ") {
                        let c = c.trim();
                        println!("GUEST_GO {c}");
                        let _ = std::fs::write("/tmp/go.tmp", c);
                        let _ = std::fs::rename("/tmp/go.tmp", "/tmp/go");
                    }
                }
            }
        }
    });
    let mut n = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        n += 1;
        if w.write_all(format!("PING {n}\n").as_bytes()).is_err() {
            println!("ping write failed at {n}");
            break;
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
