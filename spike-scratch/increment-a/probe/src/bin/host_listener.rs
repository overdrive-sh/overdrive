//! PROBE increment-a — host-side vsock listener.
//!
//! Cloud Hypervisor's vsock is HYBRID (Firecracker-derived): the host end is a
//! UNIX domain socket, not AF_VSOCK. For GUEST-initiated connections the host
//! listens on `<socket_path>_<port>` and there is NO handshake — CH just
//! forwards the stream after `accept()`. (The `CONNECT <port>\n` / `OK <port>\n`
//! handshake is the HOST-initiated direction only.)
//!
//! This process deliberately stays in the HOST network namespace for the netns
//! half of P2; it prints its own net-namespace inode so the run script can diff
//! it against the VMM's.

use std::io::Read;
use std::os::unix::net::UnixListener;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let path = std::env::args().nth(1).expect("usage: host-listener <unix-socket-path>");

    let t0 = Instant::now();
    let stamp = |t0: &Instant| {
        let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64();
        format!("t=+{:.3}s wall={:.3}", t0.elapsed().as_secs_f64(), wall)
    };

    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind unix socket");

    let netns = std::fs::read_link("/proc/self/ns/net")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|e| format!("<{e}>"));
    println!(
        "[HOST {}] listening on {path} (pid={}, netns={netns})",
        stamp(&t0),
        std::process::id()
    );

    let (mut stream, _) = listener.accept().expect("accept");
    println!("[HOST {}] accepted guest-initiated connection", stamp(&t0));

    let mut buf = [0u8; 4096];
    let mut msg_index = 0usize;
    let mut transcript = Vec::<String>::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => {
                println!("[HOST {}] EOF from guest", stamp(&t0));
                break;
            }
            Ok(n) => {
                msg_index += 1;
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                println!("[HOST {}] msg#{msg_index} ({n} bytes) = {:?}", stamp(&t0), text);
                transcript.push(text.trim_end().to_string());
            }
            Err(e) => {
                println!("[HOST {}] read error: {e}", stamp(&t0));
                break;
            }
        }
    }

    println!("[HOST {}] transcript in arrival order = {transcript:?}", stamp(&t0));

    let ordered_ok = transcript.len() >= 2
        && transcript[0].starts_with("READY")
        && transcript[1].starts_with("EXIT");
    let exit_ok = transcript.iter().any(|m| m.trim() == "EXIT 7");
    println!(
        "[HOST] separate_reads={} ordering_ready_then_exit={ordered_ok} exit_status_is_7={exit_ok}",
        transcript.len()
    );

    if ordered_ok && exit_ok {
        println!("[HOST] VERDICT: beacon + exit status received, in order, exit==7");
        std::process::exit(0);
    }
    println!("[HOST] VERDICT: FAILED expectation");
    std::process::exit(1);
}
