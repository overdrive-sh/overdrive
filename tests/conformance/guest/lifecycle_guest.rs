//! Conformance guest for the VM lifecycle-completion test.
//!
//! One dependency-free static binary the VM runs as its operator command. Its
//! first argument names the lifecycle shape the test needs:
//!
//! * `spin`: stays alive until the platform stops it (a long-lived Job).
//! * `exit0`: exits `0` at once (a finite, childless Job).
//! * `serve`: listens on TCP `18081` and, on `SIGTERM`, exits `0` (a Service
//!   that stops cooperatively inside the platform's grace period).

use std::net::TcpListener;
use std::time::Duration;

const SERVICE_PORT: u16 = 18_081;
const SIGTERM: i32 = 15;

extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn _exit(status: i32) -> !;
}

extern "C" fn exit_cleanly(_signum: i32) {
    // SAFETY: `_exit` is async-signal-safe and never returns.
    unsafe { _exit(0) }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("spin") => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
        Some("exit0") => std::process::exit(0),
        Some("serve") => {
            // SAFETY: the handler only calls the async-signal-safe `_exit`.
            unsafe { signal(SIGTERM, exit_cleanly as *const () as usize) };
            let listener =
                TcpListener::bind(("0.0.0.0", SERVICE_PORT)).expect("bind the service port");
            for connection in listener.incoming() {
                drop(connection);
            }
        }
        other => {
            eprintln!("lifecycle-guest: unknown mode {other:?}");
            std::process::exit(2);
        }
    }
}
