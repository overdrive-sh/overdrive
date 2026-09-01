use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const REQUEST: &[u8] = b"GTI_E07_REQUEST_guest_to_exec_service\n";
const RESPONSE: &[u8] = b"GTI_E07_REPLY_exec_service_to_guest\n";
const TOTAL_DEADLINE: Duration = Duration::from_secs(90);
const RESOLVE_DEADLINE: Duration = Duration::from_secs(3);
const CONNECT_DEADLINE: Duration = Duration::from_secs(3);
const IO_DEADLINE: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(500);

fn remaining(deadline: Instant) -> Option<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|duration| !duration.is_zero())
}

fn bounded(deadline: Instant, ceiling: Duration) -> Option<Duration> {
    remaining(deadline).map(|duration| duration.min(ceiling))
}

fn resolve_with_timeout(
    host: String,
    port: u16,
    timeout: Duration,
) -> std::io::Result<Vec<SocketAddr>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result =
            (host.as_str(), port).to_socket_addrs().map(|addresses| addresses.collect::<Vec<_>>());
        let _ = sender.send(result);
    });

    receiver.recv_timeout(timeout).map_err(|error| match error {
        mpsc::RecvTimeoutError::Timeout => {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "service-name resolution timed out")
        }
        mpsc::RecvTimeoutError::Disconnected => std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "service-name resolver stopped without a result",
        ),
    })?
}

fn connect(addresses: &[SocketAddr], deadline: Instant) -> std::io::Result<TcpStream> {
    let mut last_error = None;
    for address in addresses {
        let Some(timeout) = bounded(deadline, CONNECT_DEADLINE) else {
            break;
        };
        match TcpStream::connect_timeout(address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "connect deadline expired")
    }))
}

fn main() {
    let mut args = env::args().skip(1);
    let host = args.next().expect("service name argument");
    let port = args
        .next()
        .expect("service port argument")
        .parse::<u16>()
        .expect("service port must be a valid u16");
    let deadline = Instant::now() + TOTAL_DEADLINE;

    while Instant::now() < deadline {
        let Some(resolve_timeout) = bounded(deadline, RESOLVE_DEADLINE) else {
            break;
        };
        if let Ok(addresses) = resolve_with_timeout(host.clone(), port, resolve_timeout)
            && let Ok(mut stream) = connect(&addresses, deadline)
        {
            let Some(write_timeout) = bounded(deadline, IO_DEADLINE) else {
                break;
            };
            if let Err(error) = stream.set_write_timeout(Some(write_timeout)) {
                eprintln!("failed to install the bounded write timeout: {error}");
                std::process::exit(43);
            }
            if stream.write_all(REQUEST).is_ok() {
                let Some(read_timeout) = bounded(deadline, IO_DEADLINE) else {
                    break;
                };
                if let Err(error) = stream.set_read_timeout(Some(read_timeout)) {
                    eprintln!("failed to install the bounded read timeout: {error}");
                    std::process::exit(43);
                }
                let mut reply = [0_u8; RESPONSE.len()];
                if stream.read_exact(&mut reply).is_ok() && reply == RESPONSE {
                    println!("GTI_E07_EXPECTED_REPLY_RECEIVED");
                    return;
                }
            }
        }
        let Some(delay) = bounded(deadline, RETRY_DELAY) else {
            break;
        };
        thread::sleep(delay);
    }

    eprintln!("expected reply was not received before the deadline");
    std::process::exit(42);
}
