//! PROBE increment-j — host-side vsock listener for S-3.
//!
//! CH's vsock host end is a UNIX domain socket, not `AF_VSOCK` (established in
//! P2, confirmed against `cloud-hypervisor/docs/vsock.md`). A guest-initiated
//! connection to port N arrives on a listener the HOST must bind at
//! `<socket_path>_<N>`; there is no handshake in that direction.
//!
//! This is a plain sequential accept-loop, which suits both roles the probe
//! needs:
//!
//!   * port 1234 — exactly one long-lived connection the guest holds across the
//!     checkpoint. The loop parks in `read()` until it ends, and the reason it
//!     ends (clean EOF vs error) is itself the measurement.
//!   * port 1235 — one short connection per guest tick. Accept, drain, repeat.
//!
//! Every line is timestamped and flushed immediately. Flushing matters: stdout
//! is redirected to a file, so Rust block-buffers it, and a listener SIGKILLed
//! at the checkpoint would lose its entire transcript — the evidence, silently.
//!
//! The guest's payloads carry `n=<tick> nonce=<nonce>`, so what this process
//! prints is the end-to-end fact. A guest-side `send()` returning success while
//! nothing appears here is the *silently stale half-open* case, and this is the
//! only place it can be caught.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: vsock-listener <unix-socket-path> <label>");
    let label = args.next().unwrap_or_else(|| "LISTENER".into());

    let t0 = Instant::now();
    let mut out = std::io::stdout();
    macro_rules! say {
        ($($a:tt)*) => {{
            let _ = writeln!(out, "[{} t=+{:.3}s] {}", label, t0.elapsed().as_secs_f64(), format!($($a)*));
            let _ = out.flush();
        }};
    }

    // A stale socket file from a previous incarnation makes bind() fail with
    // EADDRINUSE even though nothing holds it. Remove it explicitly — a driver
    // rebinding after a checkpoint has exactly the same obligation.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            say!("BIND FAILED on {path}: {e}");
            std::process::exit(1);
        }
    };
    say!("bound {path} (pid={})", std::process::id());

    let mut conn = 0u64;
    let mut lines_total = 0u64;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                conn += 1;
                say!("ACCEPT conn#{conn}");
                let mut lines_here = 0u64;
                for line in BufReader::new(stream).lines() {
                    match line {
                        Ok(l) => {
                            lines_here += 1;
                            lines_total += 1;
                            say!("conn#{conn} <- {l}");
                        }
                        Err(e) => {
                            say!("conn#{conn} READ ERROR: {e}");
                            break;
                        }
                    }
                }
                say!("conn#{conn} CLOSED after {lines_here} lines (total {lines_total})");
            }
            Err(e) => {
                say!("ACCEPT ERROR: {e}");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}
