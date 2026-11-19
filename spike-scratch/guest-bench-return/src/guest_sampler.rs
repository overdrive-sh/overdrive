//! In-guest payload. `guest-sampler <n> console` or `guest-sampler <n> vsock <host-port>`.
//!
//! Emits `n` u64 nanosecond timings (deltas between successive `Instant::now()` reads) plus an
//! FNV-1a checksum over their little-endian bytes. Console framing:
//!   `OVDSAMPLES-BEGIN <n>` / lines of space-separated decimals / `OVDSAMPLES-END <n> <fnv hex>`
//! vsock framing: `n*8` LE bytes, then the 8-byte FNV-1a (LE). Host CID is 2.
use std::io::Write;
use std::time::Instant;

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).expect("n");
    let mode = args.get(2).map_or("console", String::as_str);
    let mut samples = Vec::with_capacity(n);
    let mut last = Instant::now();
    for _ in 0..n {
        let now = Instant::now();
        samples.push(u64::try_from(now.duration_since(last).as_nanos()).unwrap_or(u64::MAX));
        last = now;
    }
    let mut bytes = Vec::with_capacity(n * 8);
    for s in &samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    let sum = fnv(&bytes);
    match mode {
        "console" => {
            let stdout = std::io::stdout();
            let mut out = std::io::BufWriter::with_capacity(1 << 16, stdout.lock());
            writeln!(out, "OVDSAMPLES-BEGIN {n}").expect("w");
            for line in samples.chunks(16) {
                let l: Vec<String> = line.iter().map(u64::to_string).collect();
                writeln!(out, "{}", l.join(" ")).expect("w");
            }
            writeln!(out, "OVDSAMPLES-END {n} {sum:016x}").expect("w");
            out.flush().expect("flush");
        }
        "vsock" => {
            let port: u32 = args.get(3).and_then(|s| s.parse().ok()).expect("port");
            // SAFETY: plain socket/connect/write/close on an fd this block owns.
            unsafe {
                let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
                assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
                let mut addr: libc::sockaddr_vm = std::mem::zeroed();
                addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
                addr.svm_cid = 2;
                addr.svm_port = port;
                let rc = libc::connect(
                    fd,
                    (&raw const addr).cast::<libc::sockaddr>(),
                    std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
                );
                assert!(rc == 0, "connect: {}", std::io::Error::last_os_error());
                bytes.extend_from_slice(&sum.to_le_bytes());
                let mut off = 0;
                while off < bytes.len() {
                    let w = libc::write(fd, bytes[off..].as_ptr().cast(), bytes.len() - off);
                    assert!(w > 0, "write: {}", std::io::Error::last_os_error());
                    off += w as usize;
                }
                libc::shutdown(fd, libc::SHUT_WR);
                libc::close(fd);
            }
            println!("OVDSAMPLES-VSOCK-SENT {n} {sum:016x}");
        }
        other => panic!("unknown mode {other}"),
    }
}
