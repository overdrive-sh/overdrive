use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::thread;
use std::time::{Duration, Instant};

const BODY: &[u8] = b"SVM-E08-GUEST-OK";
const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(2);

fn addresses(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (host, port).to_socket_addrs().map(Iterator::collect)
}

fn receives_reply(host: &str, port: u16, http: bool) -> bool {
    let Ok(addresses) = addresses(host, port) else {
        return false;
    };
    for address in addresses {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, IO_TIMEOUT) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let request: &[u8] = if http {
            b"GET / HTTP/1.1\r\nHost: service\r\nConnection: close\r\n\r\n"
        } else {
            b"SVM_E08_REQUEST\n"
        };
        if stream.write_all(request).is_err() {
            continue;
        }
        let mut response = Vec::new();
        if stream.read_to_end(&mut response).is_ok()
            && response.windows(BODY.len()).any(|window| window == BODY)
        {
            return true;
        }
    }
    false
}

fn main() {
    let mut args = env::args().skip(1);
    let host = args.next().expect("service DNS name");
    let port = args.next().expect("service port").parse::<u16>().expect("valid service port");
    let expectation = args.next().expect("expect-reply or expect-unreachable");
    let http = args.next().as_deref() == Some("http");
    let deadline_duration = args
        .next()
        .map(|value| value.parse::<u64>().expect("deadline milliseconds"))
        .map_or(DEFAULT_DEADLINE, Duration::from_millis);
    let deadline = Instant::now() + deadline_duration;

    match expectation.as_str() {
        "expect-reply" => loop {
            if receives_reply(&host, port, http) {
                println!("SVM_CLIENT_EXACT_GUEST_REPLY_RECEIVED");
                return;
            }
            if Instant::now() >= deadline {
                eprintln!("exact VM guest reply was not received through the Service frontend");
                std::process::exit(42);
            }
            thread::sleep(Duration::from_millis(200));
        },
        "expect-unreachable" => {
            while Instant::now() < deadline {
                if receives_reply(&host, port, http) {
                    eprintln!("known-unhealthy VM backend returned the exact guest reply");
                    std::process::exit(43);
                }
                thread::sleep(Duration::from_millis(100));
            }
            println!("SVM_CLIENT_UNHEALTHY_BACKEND_NOT_REACHED");
        }
        _ => panic!("unknown expectation {expectation}"),
    }
}
