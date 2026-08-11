//! PROBE increment-d — host-side vsock collector for P6.
//!
//! Same channel shape increment-a established (CH's vsock host end is a UNIX
//! domain socket at `<socket_path>_<port>`; guest-initiated connections need no
//! handshake). This variant records EVERY line the guest sends rather than
//! expecting exactly two, because P6's guest reports a transcript of results.
//!
//! It also writes a `BEACON` marker file the moment the first message arrives,
//! so the run script can snapshot `/proc/<vmm_pid>` at a well-defined guest
//! lifecycle point — the same point in every memory-backing mode, which is what
//! makes the `shared=on` cost comparison apples-to-apples.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: host-collector <unix-socket-path> [beacon-marker-path]");
    let marker = args.next();

    let t0 = Instant::now();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind unix socket");
    println!("[HOST t=+0.000s] listening on {path} (pid={})", std::process::id());

    let (stream, _) = listener.accept().expect("accept");
    println!("[HOST t=+{:.3}s] accepted guest-initiated connection", t0.elapsed().as_secs_f64());
    if let Some(m) = &marker {
        if let Ok(mut f) = std::fs::File::create(m) {
            let _ = writeln!(f, "beacon");
        }
    }

    let mut transcript = Vec::<String>::new();
    for line in BufReader::new(stream).lines() {
        match line {
            Ok(l) => {
                println!("[HOST t=+{:.3}s] {l}", t0.elapsed().as_secs_f64());
                transcript.push(l);
            }
            Err(e) => {
                println!("[HOST t=+{:.3}s] read error: {e}", t0.elapsed().as_secs_f64());
                break;
            }
        }
    }
    println!("[HOST t=+{:.3}s] EOF from guest", t0.elapsed().as_secs_f64());

    let ready = transcript.first().map(|l| l.starts_with("READY")).unwrap_or(false);
    let exit7 = transcript.iter().any(|l| l.trim() == "EXIT 7");
    let done = transcript.iter().any(|l| l.trim() == "DONE");
    println!(
        "[HOST] lines={} ready_first={ready} exit_status_is_7={exit7} saw_DONE={done}",
        transcript.len()
    );
    if ready && exit7 && done {
        println!("[HOST] VERDICT: beacon + full result transcript + exit==7, in order");
        std::process::exit(0);
    }
    println!("[HOST] VERDICT: FAILED expectation");
    std::process::exit(1);
}
