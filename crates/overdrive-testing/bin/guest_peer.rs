//! `overdrive-guest-peer` — the guest program of the guest-kit microVM
//! fixture (GH #295, T-1; test support, never shipped).
//!
//! Two modes, chosen by the process id:
//!
//! - **PID 1 — the scripted guest vsock peer** (`guest_peer`). Mounts
//!   `/proc`, `/sys` and `/dev`, brings loopback up, loads the vsock transport
//!   modules and the guest kit's stock socket-diagnostic prerequisites plus
//!   `ovd_fwd.ko` when the image carries it,
//!   and execs the embedded production init when the workload image carries
//!   one (an explicit `overdrive.testinit=` path still takes precedence),
//!   dials the host on the port the kernel parameter `overdrive.testpeer=`
//!   names, and executes one [`PeerCommand`] per line. It runs no Overdrive
//!   owner. When the host closes the control connection it powers the guest
//!   off.
//! - **Any other pid — the guest application** (`guest_app`): one of the
//!   `guest_app::subcommand`s, run as a VM workload's operator command
//!   under the production `overdrive-init`, or as the in-guest lane's
//!   launcher (`run-with-modules`).
//!
//! Every failure is one console line; the console is the only diagnostic
//! channel a guest has.

#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "a guest program's console is its only diagnostic channel"
)]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]
#![allow(
    clippy::redundant_pub_crate,
    reason = "the binary's modules name their cross-module surface pub(crate)"
)]

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("overdrive-guest-peer runs only in a Linux guest");
    std::process::exit(64);
}

#[cfg(target_os = "linux")]
fn main() {
    // SAFETY: `getpid` has no preconditions.
    if unsafe { libc::getpid() } == 1 {
        linux::pid1::run();
    } else {
        std::process::exit(linux::app::run(&std::env::args().skip(1).collect::<Vec<_>>()));
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CString;
    use std::fs;
    use std::io;
    use std::os::fd::AsRawFd;

    /// Loads one kernel module from `path` (`finit_module`); an already
    /// loaded module is success.
    pub(crate) fn load_module(path: &str) -> io::Result<()> {
        let file = fs::File::open(path)?;
        let params = CString::default();
        // SAFETY: `file` is an open module file; `params` is a valid empty
        // C string; flags 0.
        let ret =
            unsafe { libc::syscall(libc::SYS_finit_module, file.as_raw_fd(), params.as_ptr(), 0) };
        if ret == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EEXIST) { Ok(()) } else { Err(err) }
    }

    fn raise_nofile() -> io::Result<()> {
        let nofile = libc::rlimit { rlim_cur: 65_536, rlim_max: 65_536 };
        // SAFETY: `nofile` is a valid live rlimit value; the guest fixture's
        // PID 1 owns this process limit and every child inherits it.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const nofile) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(crate) mod app {
        use std::io::{self, Read, Write};
        use std::net::{
            Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs, UdpSocket,
        };
        use std::os::fd::{AsRawFd as _, FromRawFd as _};
        use std::os::unix::process::CommandExt as _;
        use std::process::Command;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

        use overdrive_testing::guest_app::{
            Direction, FLOOD_REPORT_PERIOD_MS, FLOOD_WRITTEN_PREFIX, FLOWING_LINE,
            HOLD_FLOWING_BYTES, LISTENING_PREFIX, ListenConnEnd, STREAM_CHUNK, StreamCheck,
            StreamGen, TERMINAL_LINE_PREFIX, TerminalEvent, UDP_REPLY_TIMEOUT_MS, V6_APP_PORT,
            V6_CLIENT_ADMISSION_PORT, V6_HTTP_PROBE_PORT, V6_NAME_REQUEST, V6_NAME_RESPONSE,
            V6_POPULATION_BASE_PORT, V6_POPULATION_MAX, V6_REMOTE_REQUEST, V6_REMOTE_RESPONSE,
            V6_TCP_PROBE_PORT, V6_VIP_REQUEST, V6_VIP_RESPONSE, churn_datagram, datagram_request,
            datagram_response, exit, flood_hello, hold_hello, subcommand, tag_is_valid,
            tcp_request, tcp_response,
        };

        fn usage(detail: &str) -> i32 {
            eprintln!("overdrive-guest-peer: usage: {detail}");
            exit::USAGE
        }

        fn parse_addr(text: &str) -> Option<SocketAddrV4> {
            text.parse().ok()
        }

        /// Runs the subcommand `args[0]`; the process exit code.
        pub(crate) fn run(args: &[String]) -> i32 {
            let words: Vec<&str> = args.iter().map(String::as_str).collect();
            match words.as_slice() {
                [cmd, tcp, udp, tag] if *cmd == subcommand::ROUND_TRIP => {
                    match (parse_addr(tcp), parse_addr(udp), tag_is_valid(tag)) {
                        (Some(tcp), Some(udp), true) => round_trip(tcp, udp, tag),
                        _ => usage("round-trip <tcp ip:port> <udp ip:port> <tag>"),
                    }
                }
                [cmd, tcp, tag, rest @ ..] if *cmd == subcommand::HOLD && rest.len() <= 1 => {
                    let crash_after = match rest.first() {
                        None => None,
                        Some(arg) => match arg
                            .strip_prefix("crash-after=")
                            .and_then(|n| n.parse().ok())
                        {
                            Some(n) => Some(n),
                            None => return usage("hold <tcp ip:port> <tag> [crash-after=<bytes>]"),
                        },
                    };
                    match (parse_addr(tcp), tag_is_valid(tag)) {
                        (Some(tcp), true) => hold(tcp, tag, crash_after),
                        _ => usage("hold <tcp ip:port> <tag> [crash-after=<bytes>]"),
                    }
                }
                [cmd, tcp, tag] if *cmd == subcommand::FLOOD => {
                    match (parse_addr(tcp), tag_is_valid(tag)) {
                        (Some(tcp), true) => flood(tcp, tag),
                        _ => usage("flood <tcp ip:port> <tag>"),
                    }
                }
                [cmd, udp, count, tag] if *cmd == subcommand::UDP_CHURN => {
                    match (parse_addr(udp), count.parse::<u32>(), tag_is_valid(tag)) {
                        (Some(udp), Ok(count), true) => udp_churn(udp, count, tag),
                        _ => usage("udp-churn <udp ip:port> <count> <tag>"),
                    }
                }
                [cmd] if *cmd == subcommand::IDLE => idle(),
                [cmd, port, tag] if *cmd == subcommand::LISTEN => {
                    match (port.parse::<u16>(), tag_is_valid(tag)) {
                        (Ok(port), true) if port != 0 => listen(port),
                        _ => usage("listen <port> <tag>"),
                    }
                }
                [cmd, population] if *cmd == subcommand::V6_PEER => {
                    match population.parse::<u16>() {
                        Ok(population) if (1..=V6_POPULATION_MAX).contains(&population) => {
                            v6_peer(population)
                        }
                        _ => usage("v6-peer <population 1..=65>"),
                    }
                }
                [cmd, peer_name, peer_vip, off_tcp, off_udp, population, tag]
                    if *cmd == subcommand::V6_CLIENT =>
                {
                    match (
                        peer_vip.parse(),
                        parse_addr(off_tcp),
                        parse_addr(off_udp),
                        population.parse::<u16>(),
                        tag_is_valid(tag),
                    ) {
                        (Ok(peer_vip), Some(off_tcp), Some(off_udp), Ok(population), true)
                            if (1..=V6_POPULATION_MAX).contains(&population) =>
                        {
                            v6_client(peer_name, peer_vip, off_tcp, off_udp, population, tag)
                        }
                        _ => usage(
                            "v6-client <peer-name> <peer-vip> <off-tcp> <off-udp> \
                             <population 1..=65> <tag>",
                        ),
                    }
                }
                [cmd, rest @ ..] if *cmd == subcommand::RUN_WITH_MODULES => run_with_modules(rest),
                _ => usage("<round-trip|hold|flood|udp-churn|idle|listen|run-with-modules> …"),
            }
        }

        fn idle() -> ! {
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }

        fn round_trip(tcp: SocketAddrV4, udp: SocketAddrV4, tag: &str) -> i32 {
            let mut stream = match TcpStream::connect(tcp) {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("round-trip: tcp connect {tcp}: {e}");
                    return exit::TCP_CONNECT;
                }
            };
            let exchanged = stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .and_then(|()| stream.write_all(&tcp_request(tag)))
                .and_then(|()| stream.shutdown(Shutdown::Write))
                .and_then(|()| {
                    let mut response = Vec::new();
                    stream.read_to_end(&mut response).map(|_| response)
                });
            match exchanged {
                Ok(response) if response == tcp_response(tag) => println!("round-trip: tcp ok"),
                Ok(response) => {
                    eprintln!("round-trip: tcp response {:?}", String::from_utf8_lossy(&response));
                    return exit::TCP_MISMATCH;
                }
                Err(e) => {
                    eprintln!("round-trip: tcp exchange: {e}");
                    return exit::TCP_IO;
                }
            }
            match datagram_exchange(udp, tag) {
                Ok(()) => {
                    println!("round-trip: udp ok");
                    exit::OK
                }
                Err(code) => code,
            }
        }

        fn datagram_exchange(udp: SocketAddrV4, tag: &str) -> Result<(), i32> {
            let socket = UdpSocket::bind("0.0.0.0:0")
                .and_then(|s| s.connect(udp).map(|()| s))
                .map_err(|e| {
                    eprintln!("udp: socket for {udp}: {e}");
                    exit::UDP_IO
                })?;
            socket
                .set_read_timeout(Some(Duration::from_millis(UDP_REPLY_TIMEOUT_MS)))
                .map_err(|_| exit::UDP_IO)?;
            socket.send(&[]).map_err(|e| {
                eprintln!("udp: send empty to {udp}: {e}");
                exit::UDP_IO
            })?;
            let mut empty = [0u8; 1];
            match socket.recv(&mut empty) {
                Ok(0) => println!("round-trip: udp-empty ok"),
                Ok(n) => {
                    eprintln!("udp: empty reply carried {n} bytes");
                    return Err(exit::UDP_MISMATCH);
                }
                Err(e) => {
                    eprintln!("udp: empty reply: {e}");
                    return Err(exit::UDP_TIMEOUT);
                }
            }
            socket.send(&datagram_request(tag)).map_err(|e| {
                eprintln!("udp: send to {udp}: {e}");
                exit::UDP_IO
            })?;
            let mut buf = [0u8; 2048];
            match socket.recv(&mut buf) {
                Ok(n) if buf[..n] == datagram_response(tag)[..] => Ok(()),
                Ok(n) => {
                    eprintln!("udp: reply {:?}", String::from_utf8_lossy(&buf[..n]));
                    Err(exit::UDP_MISMATCH)
                }
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
                {
                    eprintln!("udp: no reply within {UDP_REPLY_TIMEOUT_MS} ms");
                    Err(exit::UDP_TIMEOUT)
                }
                Err(e) => {
                    eprintln!("udp: recv: {e}");
                    Err(exit::UDP_IO)
                }
            }
        }

        fn v6_peer(population: u16) -> i32 {
            let healthy = Arc::new(AtomicBool::new(true));
            let app = bind_v6_tcp(V6_APP_PORT);
            let http = bind_v6_tcp(V6_HTTP_PROBE_PORT);
            let mut population_listeners = Vec::with_capacity(usize::from(population));
            for index in 0..population {
                let port = V6_POPULATION_BASE_PORT + index;
                population_listeners.push((index, bind_v6_tcp(port)));
            }

            {
                let healthy = Arc::clone(&healthy);
                thread::spawn(move || v6_app_accept(&app, &healthy));
            }
            {
                let healthy = Arc::clone(&healthy);
                thread::spawn(move || v6_http_accept(&http, &healthy));
            }
            {
                let healthy = Arc::clone(&healthy);
                thread::spawn(move || v6_tcp_probe_accept(&healthy));
            }
            for (index, listener) in population_listeners {
                thread::spawn(move || v6_population_accept(index, &listener));
            }
            println!("v6-peer: ready population={population}");
            idle()
        }

        fn bind_v6_tcp(port: u16) -> TcpListener {
            TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
                .unwrap_or_else(|e| panic!("v6-peer: bind TCP {port}: {e}"))
        }

        fn v6_app_accept(listener: &TcpListener, healthy: &Arc<AtomicBool>) {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let healthy = Arc::clone(healthy);
                        thread::spawn(move || v6_app_connection(stream, &healthy));
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => eprintln!("v6-peer: app accept: {e}"),
                }
            }
        }

        fn v6_app_connection(mut stream: TcpStream, healthy: &AtomicBool) {
            let _ = stream.set_nodelay(true);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            let request = match read_line_bytes(&mut stream, 256) {
                Ok(request) => request,
                Err(e) => {
                    eprintln!("v6-peer: app read: {e}");
                    return;
                }
            };
            let response = if request == V6_NAME_REQUEST {
                V6_NAME_RESPONSE
            } else if request == V6_VIP_REQUEST {
                V6_VIP_RESPONSE
            } else if request == V6_REMOTE_REQUEST {
                V6_REMOTE_RESPONSE
            } else {
                eprintln!("v6-peer: unexpected app request {request:?}");
                return;
            };
            if let Err(e) = stream.write_all(response) {
                eprintln!("v6-peer: app reply: {e}");
                return;
            }
            println!("v6-peer: app request={}", String::from_utf8_lossy(&request).trim());
            if request == V6_VIP_REQUEST {
                let _ = stream.set_read_timeout(None);
                v6_state_control(&mut stream, healthy);
                return;
            }
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }

        fn v6_state_control(stream: &mut TcpStream, healthy: &AtomicBool) {
            let command = match read_line_bytes(stream, 64) {
                Ok(command) => command,
                Err(e) => {
                    eprintln!("v6-peer: state control read: {e}");
                    return;
                }
            };
            let command = std::str::from_utf8(&command).unwrap_or("").trim();
            if command != "V6-STATE:unhealthy" {
                eprintln!("v6-peer: bad state command {command:?}");
                return;
            }
            healthy.store(false, Ordering::SeqCst);
            let _ = stream.write_all(b"V6-STATE-ACK\n");
            println!("v6-peer: state=unhealthy");
            thread::sleep(Duration::from_secs(12));
            healthy.store(true, Ordering::SeqCst);
            println!("v6-peer: state=healthy");
        }

        fn v6_http_accept(listener: &TcpListener, healthy: &Arc<AtomicBool>) {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                        if let Err(e) = consume_http_request_head(&mut stream) {
                            eprintln!("v6-peer: HTTP request read: {e}");
                            continue;
                        }
                        let pass = healthy.load(Ordering::SeqCst);
                        let response = if pass {
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK"
                                .as_slice()
                        } else {
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nFAIL"
                                .as_slice()
                        };
                        let _ = stream.write_all(response);
                        let _ = stream.shutdown(Shutdown::Write);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => eprintln!("v6-peer: http accept: {e}"),
                }
            }
        }

        fn consume_http_request_head(stream: &mut TcpStream) -> io::Result<()> {
            const MAX_HEAD_BYTES: usize = 8 * 1024;
            let mut request = Vec::with_capacity(512);
            let mut chunk = [0u8; 512];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                if request.len() == MAX_HEAD_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTTP request head exceeds 8192 bytes",
                    ));
                }
                let remaining = MAX_HEAD_BYTES - request.len();
                let read_len = remaining.min(chunk.len());
                let received = stream.read(&mut chunk[..read_len])?;
                if received == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "HTTP request ended before its header terminator",
                    ));
                }
                request.extend_from_slice(&chunk[..received]);
            }
            Ok(())
        }

        fn v6_tcp_probe_accept(healthy: &Arc<AtomicBool>) {
            let mut listener: Option<TcpListener> = None;
            loop {
                if healthy.load(Ordering::SeqCst) {
                    if listener.is_none() {
                        match bind_reusable_tcp(V6_TCP_PROBE_PORT) {
                            Ok(bound) => {
                                if let Err(e) = bound.set_nonblocking(true) {
                                    eprintln!("v6-peer: TCP probe nonblocking: {e}");
                                }
                                listener = Some(bound);
                            }
                            Err(e) => eprintln!("v6-peer: TCP probe rebind: {e}"),
                        }
                    }
                    if let Some(bound) = &listener {
                        match bound.accept() {
                            Ok((_stream, _)) => {}
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(e) => eprintln!("v6-peer: TCP probe accept: {e}"),
                        }
                    }
                } else {
                    listener = None;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }

        fn bind_reusable_tcp(port: u16) -> io::Result<TcpListener> {
            let fd =
                unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let listener = unsafe { TcpListener::from_raw_fd(fd) };
            let one: libc::c_int = 1;
            let option_len = libc::socklen_t::try_from(std::mem::size_of_val(&one))
                .map_err(|_| io::Error::other("socket option size exceeds socklen_t"))?;
            if unsafe {
                libc::setsockopt(
                    listener.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_REUSEADDR,
                    std::ptr::from_ref(&one).cast(),
                    option_len,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let family = u16::try_from(libc::AF_INET)
                .map_err(|_| io::Error::other("AF_INET exceeds sa_family_t"))?;
            let address = libc::sockaddr_in {
                sin_family: family,
                sin_port: port.to_be(),
                sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes([0, 0, 0, 0]) },
                sin_zero: [0; 8],
            };
            let address_len = libc::socklen_t::try_from(std::mem::size_of_val(&address))
                .map_err(|_| io::Error::other("socket address size exceeds socklen_t"))?;
            if unsafe {
                libc::bind(listener.as_raw_fd(), std::ptr::from_ref(&address).cast(), address_len)
            } != 0
                || unsafe { libc::listen(listener.as_raw_fd(), 128) } != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(listener)
        }

        fn v6_population_accept(index: u16, listener: &TcpListener) {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let local = socket_context(stream.local_addr());
                        let peer = socket_context(stream.peer_addr());
                        println!(
                            "v6-peer: population index={index} port={} local={local} peer={peer} \
                             stage=ACCEPT",
                            V6_POPULATION_BASE_PORT + index
                        );
                        thread::spawn(move || {
                            v6_population_connection(index, stream, &local, &peer);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => eprintln!("v6-peer: population {index} accept: {e}"),
                }
            }
        }

        fn v6_population_connection(index: u16, mut stream: TcpStream, local: &str, peer: &str) {
            let port = V6_POPULATION_BASE_PORT + index;
            let expected = format!("V6-POP:{index}\n");
            let mut got = vec![0u8; expected.len()];
            if let Err(error) = stream.read_exact(&mut got) {
                eprintln!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=READ errno={} error={error}",
                    errno_of(&error)
                );
                println!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=request-read-error"
                );
                return;
            }
            if got != expected.as_bytes() {
                eprintln!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=REQUEST errno=none expected={:?} got={got:?}",
                    expected.as_bytes()
                );
                println!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=request-mismatch"
                );
                return;
            }
            println!(
                "v6-peer: population index={index} port={port} local={local} peer={peer} \
                 stage=REQUEST bytes={} result=ok",
                got.len()
            );
            let ack = format!("V6-POP-ACK:{index}\n");
            if let Err(error) = stream.write_all(ack.as_bytes()) {
                eprintln!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=ACK errno={} error={error}",
                    errno_of(&error)
                );
                println!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=ack-write-error"
                );
                return;
            }
            println!(
                "v6-peer: population index={index} port={port} local={local} peer={peer} \
                 stage=ACK bytes={} result=ok",
                ack.len()
            );
            let mut closure = [0u8; 1];
            match stream.read(&mut closure) {
                Ok(0) => println!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=eof errno=none"
                ),
                Ok(n) => eprintln!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=unexpected-data errno=none bytes={n} data={:?}",
                    &closure[..n]
                ),
                Err(error) => eprintln!(
                    "v6-peer: population index={index} port={port} local={local} peer={peer} \
                     stage=CLOSE cause=read-error errno={} error={error}",
                    errno_of(&error)
                ),
            }
        }

        fn socket_context(address: io::Result<std::net::SocketAddr>) -> String {
            address
                .map_or_else(|error| format!("unavailable({error})"), |address| address.to_string())
        }

        fn errno_of(error: &io::Error) -> String {
            error.raw_os_error().map_or_else(|| "none".to_owned(), |errno| errno.to_string())
        }

        fn v6_client(
            peer_name: &str,
            peer_vip: Ipv4Addr,
            off_tcp: SocketAddrV4,
            off_udp: SocketAddrV4,
            population: u16,
            tag: &str,
        ) -> i32 {
            let admission = TcpListener::bind((Ipv4Addr::UNSPECIFIED, V6_CLIENT_ADMISSION_PORT))
                .unwrap_or_else(|e| panic!("v6-client: bind admission listener: {e}"));
            thread::spawn(move || {
                for stream in admission.incoming() {
                    drop(stream);
                }
            });

            let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
            let nameserver =
                resolv.lines().find_map(|line| line.strip_prefix("nameserver ")).unwrap_or("");
            println!("v6-client: nameserver={nameserver}");
            // The Service's workload SVID is converged independently of VM
            // process start. Let the production identity lifecycle publish it
            // before the first mesh handshake; subsequent retries remain the
            // fail-closed bound for real transport failures.
            thread::sleep(Duration::from_secs(10));

            if tcp_exchange(off_tcp, &tcp_request(tag), &tcp_response(tag)).is_err() {
                return exit::TCP_IO;
            }
            if datagram_exchange(off_udp, tag).is_err() {
                return exit::UDP_IO;
            }
            println!("v6-client: off-node ok");

            let name_addr = match retry_for(Duration::from_secs(30), || {
                (peer_name, V6_APP_PORT)
                    .to_socket_addrs()?
                    .find_map(|addr| match addr {
                        std::net::SocketAddr::V4(addr) => Some(addr),
                        std::net::SocketAddr::V6(_) => None,
                    })
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::AddrNotAvailable, "no IPv4 answer")
                    })
            }) {
                Ok(addr) => addr,
                Err(error) => {
                    eprintln!("v6-client: resolve {peer_name}: {error}");
                    return exit::TCP_CONNECT;
                }
            };
            println!("v6-client: resolved {peer_name}={}", name_addr.ip());
            if let Err(error) = retry_for(Duration::from_secs(30), || {
                tcp_exchange(name_addr, V6_NAME_REQUEST, V6_NAME_RESPONSE)
            }) {
                eprintln!("v6-client: mesh-name TCP did not become ready: {error}");
                return exit::TCP_IO;
            }

            let mut vip_stream =
                match retry_for(Duration::from_secs(30), || open_vip_stream(peer_vip)) {
                    Ok(stream) => stream,
                    Err(e) => {
                        eprintln!("v6-client: VIP TCP did not become ready: {e}");
                        return exit::TCP_CONNECT;
                    }
                };
            println!("v6-client: vip-tcp ok");

            println!("v6-client: paths-ready");
            thread::sleep(Duration::from_secs(5));
            if send_state(&mut vip_stream).is_err() {
                return exit::TCP_IO;
            }
            println!("v6-client: state=unhealthy");
            drop(vip_stream);
            thread::sleep(Duration::from_secs(12));
            let vip_stream = match retry_for(Duration::from_secs(30), || open_vip_stream(peer_vip))
            {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("v6-client: restored VIP TCP did not become ready: {e}");
                    return exit::TCP_CONNECT;
                }
            };
            println!("v6-client: state=healthy");
            thread::sleep(Duration::from_secs(5));

            let population_streams = match hold_population(peer_vip, population) {
                Ok(streams) => streams,
                Err(e) => {
                    eprintln!("v6-client: population: {e}");
                    return exit::TCP_IO;
                }
            };
            println!("v6-client: population-held={population}");
            let _held = (vip_stream, population_streams);
            idle()
        }

        fn open_vip_stream(peer_vip: Ipv4Addr) -> io::Result<TcpStream> {
            let mut stream = TcpStream::connect((peer_vip, V6_APP_PORT))?;
            stream.set_nodelay(true)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(V6_VIP_REQUEST)?;
            read_exact_record(&mut stream, V6_VIP_RESPONSE)?;
            Ok(stream)
        }

        fn hold_population(
            peer_vip: Ipv4Addr,
            population: u16,
        ) -> io::Result<Arc<Mutex<Vec<TcpStream>>>> {
            let streams = Arc::new(Mutex::new(Vec::with_capacity(usize::from(population))));
            let population_deadline = std::time::Instant::now() + Duration::from_secs(30);
            for index in 0..population {
                let request = format!("V6-POP:{index}\n");
                let response = format!("V6-POP-ACK:{index}\n");
                let target = SocketAddrV4::new(peer_vip, V6_POPULATION_BASE_PORT + index);
                let stream = retry_until(population_deadline, || {
                    let mut stream = TcpStream::connect(target)
                        .map_err(|error| population_error(index, target, "CONNECT", &error))?;
                    stream
                        .write_all(request.as_bytes())
                        .map_err(|error| population_error(index, target, "WRITE", &error))?;
                    let mut got = vec![0u8; response.len()];
                    stream
                        .read_exact(&mut got)
                        .map_err(|error| population_error(index, target, "READ", &error))?;
                    if got != response.as_bytes() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "population index={index} target={target} stage=ACK errno=none \
                                 expected={:?} got={got:?}",
                                response.as_bytes()
                            ),
                        ));
                    }
                    Ok(stream)
                })?;
                streams.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(stream);
            }
            Ok(streams)
        }

        fn population_error(
            index: u16,
            target: SocketAddrV4,
            stage: &'static str,
            error: &io::Error,
        ) -> io::Error {
            let kind = error.kind();
            let errno = errno_of(error);
            io::Error::new(
                kind,
                format!(
                    "population index={index} target={target} stage={stage} errno={errno} \
                     error={error}"
                ),
            )
        }

        fn retry_for<T>(
            bound: Duration,
            operation: impl FnMut() -> io::Result<T>,
        ) -> io::Result<T> {
            retry_until(std::time::Instant::now() + bound, operation)
        }

        fn retry_until<T>(
            deadline: std::time::Instant,
            mut operation: impl FnMut() -> io::Result<T>,
        ) -> io::Result<T> {
            loop {
                match operation() {
                    Ok(value) => return Ok(value),
                    Err(_error) if std::time::Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(200));
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        fn tcp_exchange(addr: SocketAddrV4, request: &[u8], response: &[u8]) -> io::Result<()> {
            let mut stream = TcpStream::connect(addr)?;
            stream.set_nodelay(true)?;
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            stream.write_all(request)?;
            read_exact_record(&mut stream, response)
        }

        fn read_exact_record(stream: &mut TcpStream, expected: &[u8]) -> io::Result<()> {
            let mut got = vec![0u8; expected.len()];
            stream.read_exact(&mut got)?;
            if got == expected {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::InvalidData, "byte-distinct reply mismatch"))
            }
        }

        fn read_line_bytes(stream: &mut TcpStream, cap: usize) -> io::Result<Vec<u8>> {
            let mut line = Vec::new();
            while line.len() < cap {
                let mut byte = [0u8; 1];
                stream.read_exact(&mut byte)?;
                line.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(line);
                }
            }
            Err(io::Error::new(io::ErrorKind::InvalidData, "line exceeds fixture bound"))
        }

        fn send_state(stream: &mut TcpStream) -> io::Result<()> {
            stream.write_all(b"V6-STATE:unhealthy\n")?;
            read_exact_record(stream, b"V6-STATE-ACK\n")
        }

        /// `listen <port> <tag>` (U-D17-DIAL): accepts until killed; one
        /// thread per connection.
        fn listen(port: u16) -> i32 {
            let listener = match TcpListener::bind(("0.0.0.0", port)) {
                Ok(listener) => listener,
                Err(e) => {
                    eprintln!("listen: bind 0.0.0.0:{port}: {e}");
                    return exit::TCP_CONNECT;
                }
            };
            println!("{LISTENING_PREFIX}{port}");
            let mut accepted: u64 = 0;
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        accepted += 1;
                        let conn = accepted;
                        thread::spawn(move || serve_listen_conn(conn, stream));
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        eprintln!("listen: accept: {e}");
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        }

        /// Serves one accepted connection: a `hold` client gets the
        /// peer-to-guest stream back and its own stream verified; anything
        /// else (a probe that connects and drops) is read until it ends.
        /// Prints the connection's end line.
        fn serve_listen_conn(conn: u64, mut stream: TcpStream) {
            let mut buf = vec![0u8; STREAM_CHUNK];
            let mut first_line: Vec<u8> = Vec::new();
            let mut header_done = false;
            let mut check: Option<StreamCheck> = None;
            let terminal = loop {
                match stream.read(&mut buf) {
                    Ok(0) => break TerminalEvent::Eof,
                    Ok(n) => {
                        let mut chunk = &buf[..n];
                        if !header_done {
                            let Some(at) = chunk.iter().position(|b| *b == b'\n') else {
                                first_line.extend_from_slice(chunk);
                                continue;
                            };
                            first_line.extend_from_slice(&chunk[..=at]);
                            chunk = &chunk[at + 1..];
                            header_done = true;
                            if let Some(tag) = hold_tag(&first_line) {
                                start_listen_writer(&stream, &tag);
                                check = Some(StreamCheck::new(&tag, Direction::GuestToPeer));
                            }
                        }
                        if let Some(check) = check.as_mut()
                            && let Err(at) = check.check(chunk)
                        {
                            eprintln!(
                                "listen: conn {conn}: the client's stream is corrupt at byte {at}"
                            );
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => break TerminalEvent::from_read_error(&e),
                }
            };
            println!("{}", ListenConnEnd { conn, hello: check.is_some(), terminal });
        }

        /// The tag of a `hold` hello line, when `line` is exactly one.
        fn hold_tag(line: &[u8]) -> Option<String> {
            let text = std::str::from_utf8(line).ok()?;
            let tag = text.strip_prefix("HOLD:")?.strip_suffix('\n')?;
            (tag_is_valid(tag) && hold_hello(tag) == line).then(|| tag.to_owned())
        }

        /// Writes the peer-to-guest stream of `tag` on a clone of `stream`
        /// until a write fails.
        fn start_listen_writer(stream: &TcpStream, tag: &str) {
            let Ok(mut writer) = stream.try_clone() else {
                eprintln!("listen: clone for the writer failed");
                return;
            };
            let tag = tag.to_owned();
            thread::spawn(move || {
                let mut stream_gen = StreamGen::new(&tag, Direction::PeerToGuest);
                let mut chunk = vec![0u8; STREAM_CHUNK];
                loop {
                    stream_gen.fill(&mut chunk);
                    if writer.write_all(&chunk).is_err() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            });
        }

        fn crash_kernel() {
            // SAFETY: `sync` has no preconditions.
            unsafe { libc::sync() };
            let _ = std::fs::write("/proc/sysrq-trigger", b"c");
        }

        fn hold(tcp: SocketAddrV4, tag: &str, crash_after: Option<u64>) -> i32 {
            let mut stream = match TcpStream::connect(tcp) {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("hold: tcp connect {tcp}: {e}");
                    return exit::TCP_CONNECT;
                }
            };
            if let Err(e) = stream.write_all(&hold_hello(tag)) {
                eprintln!("hold: hello: {e}");
                return exit::TCP_IO;
            }
            let written = Arc::new(AtomicU64::new(0));
            let mut writer = match stream.try_clone() {
                Ok(writer) => writer,
                Err(e) => {
                    eprintln!("hold: clone: {e}");
                    return exit::TCP_IO;
                }
            };
            let writer_tag = tag.to_owned();
            let writer_count = Arc::clone(&written);
            thread::spawn(move || {
                let mut stream_gen = StreamGen::new(&writer_tag, Direction::GuestToPeer);
                let mut chunk = vec![0u8; STREAM_CHUNK];
                loop {
                    stream_gen.fill(&mut chunk);
                    if writer.write_all(&chunk).is_err() {
                        return;
                    }
                    writer_count.fetch_add(chunk.len() as u64, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(2));
                }
            });
            let mut check = StreamCheck::new(tag, Direction::PeerToGuest);
            let mut buf = vec![0u8; STREAM_CHUNK];
            let mut flowing = false;
            let terminal = loop {
                match stream.read(&mut buf) {
                    Ok(0) => break TerminalEvent::Eof,
                    Ok(n) => {
                        if let Err(at) = check.check(&buf[..n]) {
                            eprintln!("hold: the peer's stream is corrupt at byte {at}");
                        }
                        let both = check.verified().min(written.load(Ordering::SeqCst));
                        if !flowing && both >= HOLD_FLOWING_BYTES {
                            flowing = true;
                            println!("{FLOWING_LINE}");
                        }
                        if crash_after.is_some_and(|n| both >= n) {
                            println!("hold: crashing the guest kernel");
                            crash_kernel();
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => break TerminalEvent::from_read_error(&e),
                }
            };
            println!("{TERMINAL_LINE_PREFIX}{terminal}");
            idle()
        }

        fn flood(tcp: SocketAddrV4, tag: &str) -> i32 {
            let mut stream = match TcpStream::connect(tcp) {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("flood: tcp connect {tcp}: {e}");
                    return exit::TCP_CONNECT;
                }
            };
            if let Err(e) = stream.write_all(&flood_hello(tag)) {
                eprintln!("flood: hello: {e}");
                return exit::TCP_IO;
            }
            let written = Arc::new(AtomicU64::new(0));
            let stopped = Arc::new(AtomicBool::new(false));
            let reporter_count = Arc::clone(&written);
            let reporter_stop = Arc::clone(&stopped);
            thread::spawn(move || {
                while !reporter_stop.load(Ordering::SeqCst) {
                    println!("{FLOOD_WRITTEN_PREFIX}{}", reporter_count.load(Ordering::SeqCst));
                    thread::sleep(Duration::from_millis(FLOOD_REPORT_PERIOD_MS));
                }
            });
            let mut stream_gen = StreamGen::new(tag, Direction::GuestToPeer);
            let mut chunk = vec![0u8; STREAM_CHUNK];
            let terminal = loop {
                stream_gen.fill(&mut chunk);
                match stream.write_all(&chunk) {
                    Ok(()) => {
                        written.fetch_add(chunk.len() as u64, Ordering::SeqCst);
                    }
                    Err(e) => break TerminalEvent::from_read_error(&e),
                }
            };
            println!("{TERMINAL_LINE_PREFIX}{terminal}");
            idle()
        }

        fn udp_churn(udp: SocketAddrV4, count: u32, tag: &str) -> i32 {
            for index in 0..count {
                let sent = UdpSocket::bind("0.0.0.0:0")
                    .and_then(|s| s.connect(udp).map(|()| s))
                    .and_then(|s| s.send(&churn_datagram(tag, index)));
                if let Err(e) = sent {
                    eprintln!("udp-churn: send {index} of {count}: {e}");
                    return exit::CHURN_SEND;
                }
            }
            println!("udp-churn: {count} sockets sent and closed");
            match datagram_exchange(udp, tag) {
                Ok(()) => {
                    println!("udp-churn: round trip ok");
                    exit::OK
                }
                Err(code) => code,
            }
        }

        fn run_with_modules(args: &[&str]) -> i32 {
            let Some(split) = args.iter().position(|a| *a == "--") else {
                return usage("run-with-modules <module.ko>... -- <program> <args>...");
            };
            for module in &args[..split] {
                if let Err(e) = super::load_module(module) {
                    eprintln!("run-with-modules: loading {module}: {e}");
                    return exit::MODULE_LOAD;
                }
            }
            let Some((program, rest)) = args[split + 1..].split_first() else {
                return usage("run-with-modules <module.ko>... -- <program> <args>...");
            };
            if let Err(e) = super::raise_nofile() {
                eprintln!("run-with-modules: raising RLIMIT_NOFILE: {e}");
                return exit::EXEC;
            }
            let error = Command::new(program)
                .args(rest)
                .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
                .exec();
            eprintln!("run-with-modules: exec {program}: {error}");
            exit::EXEC
        }
    }

    pub(crate) mod pid1 {
        use std::collections::BTreeMap;
        use std::ffi::CString;
        use std::io::{self, Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
        use std::os::unix::process::CommandExt as _;
        use std::path::Path;
        use std::process::Command;
        use std::sync::{Arc, Mutex};
        use std::thread::{self, JoinHandle};
        use std::time::{Duration, Instant};

        use overdrive_core::guest_transport::GUEST_FORWARDER_MODULE_PATH;
        use overdrive_core::guest_transport::forwarder::{
            AbortCause, AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome,
            PairMode, PairSide,
        };
        use overdrive_core::vm::beacon::{GUEST_VSOCK_MODULE_DIR, GUEST_VSOCK_MODULE_FILES};
        use overdrive_fwd::KernelFlowForwarder;
        use overdrive_testing::guest_app::{
            Direction, STREAM_CHUNK, StreamCheck, StreamGen, TerminalEvent,
        };
        use overdrive_testing::guest_kit::{
            DUMMY_MODULE_GUEST_PATH, LOOPBACK_MODULE_GUEST_PATH, NETEM_MODULE_GUEST_PATH,
            OVERDRIVE_INIT_GUEST_PATH, PRIO_MODULE_GUEST_PATH, TEST_INIT_PARAM,
        };
        use overdrive_testing::guest_peer::{
            CONTROL_PORT_PARAM, DrainEnd, PeerCommand, PeerEnd, PeerOutcome, PeerReply, PeerSide,
        };
        use overdrive_testing::vsock::{
            VMADDR_CID_ANY, VMADDR_CID_HOST, VsockListener, VsockStream, wait_ready,
        };

        enum Resource {
            Vsock(VsockStream),
            VsockListener(VsockListener),
            Tcp(TcpStream),
            TcpListener(TcpListener),
            Forwarder(KernelFlowForwarder),
        }

        impl AsFd for Resource {
            fn as_fd(&self) -> BorrowedFd<'_> {
                match self {
                    Self::Vsock(s) => s.as_fd(),
                    Self::VsockListener(l) => l.as_fd(),
                    Self::Tcp(s) => s.as_fd(),
                    Self::TcpListener(l) => l.as_fd(),
                    Self::Forwarder(f) => f.readiness_fd(),
                }
            }
        }

        #[derive(Default)]
        struct State {
            resources: Mutex<BTreeMap<String, Arc<Resource>>>,
            tasks: Mutex<BTreeMap<String, JoinHandle<PeerReply>>>,
        }

        fn err(detail: impl Into<String>, error: &io::Error) -> PeerReply {
            PeerReply::Err {
                errno: error.raw_os_error().unwrap_or(-1),
                detail: format!("{}: {error}", detail.into()),
            }
        }

        fn failure(detail: impl Into<String>) -> PeerReply {
            PeerReply::Err { errno: -1, detail: detail.into() }
        }

        impl State {
            fn get(&self, handle: &str) -> Result<Arc<Resource>, PeerReply> {
                self.resources
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(handle)
                    .cloned()
                    .ok_or_else(|| failure(format!("no resource {handle:?}")))
            }

            fn put(&self, handle: String, resource: Resource) -> PeerReply {
                let mut map =
                    self.resources.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if map.contains_key(&handle) {
                    return failure(format!("resource {handle:?} exists"));
                }
                map.insert(handle, Arc::new(resource));
                PeerReply::Ok
            }

            fn take(&self, handle: &str) -> Option<Arc<Resource>> {
                self.resources
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(handle)
            }
        }

        fn mount(source: &str, target: &str, fstype: &str) {
            let (Ok(source), Ok(target), Ok(fstype)) =
                (CString::new(source), CString::new(target), CString::new(fstype))
            else {
                return;
            };
            // SAFETY: valid C strings; no data argument.
            let ret = unsafe {
                libc::mount(source.as_ptr(), target.as_ptr(), fstype.as_ptr(), 0, std::ptr::null())
            };
            if ret != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EBUSY) {
                    eprintln!("overdrive-guest-peer: mount {fstype:?} on {target:?}: {error}");
                }
            }
        }

        fn bring_loopback_up() -> io::Result<()> {
            // SAFETY: `socket` has no pointer arguments; the returned descriptor is
            // checked before it is transferred to `OwnedFd` exactly once.
            let raw =
                unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `raw` is a newly opened descriptor owned by this function.
            let socket = unsafe { OwnedFd::from_raw_fd(raw) };
            // SAFETY: all-zero is a valid initial representation for `ifreq`.
            let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
            for (slot, byte) in request.ifr_name.iter_mut().zip("lo".bytes()) {
                *slot = libc::c_char::from_ne_bytes([byte]);
            }
            #[cfg(target_env = "musl")]
            let get_flags = libc::c_int::try_from(libc::SIOCGIFFLAGS)
                .map_err(|_| io::Error::other("SIOCGIFFLAGS does not fit ioctl request"))?;
            #[cfg(not(target_env = "musl"))]
            let get_flags = libc::SIOCGIFFLAGS;
            // SAFETY: `socket` is an open AF_INET datagram descriptor and `request`
            // is a live, correctly sized `ifreq` naming `lo`.
            let read = unsafe { libc::ioctl(socket.as_raw_fd(), get_flags, &raw mut request) };
            if read != 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: SIOCGIFFLAGS initialized the flags member of the union.
            let flags = unsafe { request.ifr_ifru.ifru_flags };
            let up = libc::c_short::try_from(libc::IFF_UP)
                .map_err(|_| io::Error::other("IFF_UP does not fit ifreq flags"))?;
            request.ifr_ifru.ifru_flags = flags | up;
            #[cfg(target_env = "musl")]
            let set_flags = libc::c_int::try_from(libc::SIOCSIFFLAGS)
                .map_err(|_| io::Error::other("SIOCSIFFLAGS does not fit ioctl request"))?;
            #[cfg(not(target_env = "musl"))]
            let set_flags = libc::SIOCSIFFLAGS;
            // SAFETY: the same open socket and live `ifreq` now carry the existing
            // flags with only IFF_UP added.
            let written = unsafe { libc::ioctl(socket.as_raw_fd(), set_flags, &raw mut request) };
            if written == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
        }

        fn power_off() -> ! {
            // SAFETY: `sync` and `reboot` have no memory preconditions.
            unsafe {
                libc::sync();
                libc::reboot(libc::RB_POWER_OFF);
            }
            loop {
                thread::sleep(Duration::from_secs(3600));
            }
        }

        fn control_port() -> Option<u32> {
            let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
            cmdline
                .split_whitespace()
                .find_map(|p| p.strip_prefix(CONTROL_PORT_PARAM)?.parse().ok())
        }

        fn test_init() -> Option<String> {
            let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
            cmdline
                .split_whitespace()
                .find_map(|parameter| parameter.strip_prefix(TEST_INIT_PARAM).map(str::to_owned))
        }

        fn production_init() -> Option<String> {
            test_init().or_else(|| {
                Path::new(OVERDRIVE_INIT_GUEST_PATH)
                    .is_file()
                    .then(|| OVERDRIVE_INIT_GUEST_PATH.to_owned())
            })
        }

        const SOCKET_DIAGNOSTIC_MODULES: [&str; 3] = [
            "/lib/overdrive-testing/modules/inet_diag.ko",
            "/lib/overdrive-testing/modules/tcp_diag.ko",
            "/lib/overdrive-testing/modules/udp_diag.ko",
        ];

        fn load_production_fixture_prerequisites() -> Result<(), (&'static str, io::Error)> {
            for prerequisite in [DUMMY_MODULE_GUEST_PATH, LOOPBACK_MODULE_GUEST_PATH]
                .into_iter()
                .chain(SOCKET_DIAGNOSTIC_MODULES)
            {
                if std::path::Path::new(prerequisite).exists() {
                    super::load_module(prerequisite).map_err(|error| (prerequisite, error))?;
                }
            }
            Ok(())
        }

        /// PID 1 of a scripted guest.
        pub(crate) fn run() -> ! {
            mount("proc", "/proc", "proc");
            mount("sysfs", "/sys", "sysfs");
            mount("devtmpfs", "/dev", "devtmpfs");
            if let Err(e) = bring_loopback_up() {
                eprintln!("overdrive-guest-peer: bringing loopback up: {e}");
                power_off();
            }
            for file in GUEST_VSOCK_MODULE_FILES {
                let path = format!("{GUEST_VSOCK_MODULE_DIR}/{file}");
                if std::path::Path::new(&path).exists()
                    && let Err(e) = super::load_module(&path)
                {
                    eprintln!("overdrive-guest-peer: loading {path}: {e}");
                }
            }
            if let Some(program) = production_init() {
                if let Err(e) = super::raise_nofile() {
                    eprintln!(
                        "overdrive-guest-peer: raising production fixture RLIMIT_NOFILE: {e}"
                    );
                    power_off();
                }
                if let Err((prerequisite, error)) = load_production_fixture_prerequisites() {
                    eprintln!(
                        "overdrive-guest-peer: loading production fixture prerequisite \
                         {prerequisite}: {error}"
                    );
                    power_off();
                }
                let error = Command::new(&program).exec();
                eprintln!("overdrive-guest-peer: exec {program}: {error}");
                power_off();
            }
            for optional in [
                GUEST_FORWARDER_MODULE_PATH,
                LOOPBACK_MODULE_GUEST_PATH,
                DUMMY_MODULE_GUEST_PATH,
                PRIO_MODULE_GUEST_PATH,
                NETEM_MODULE_GUEST_PATH,
            ]
            .into_iter()
            .chain(SOCKET_DIAGNOSTIC_MODULES)
            {
                if std::path::Path::new(optional).exists()
                    && let Err(e) = super::load_module(optional)
                {
                    eprintln!("overdrive-guest-peer: loading {optional}: {e}");
                }
            }
            let Some(port) = control_port() else {
                eprintln!(
                    "overdrive-guest-peer: no {CONTROL_PORT_PARAM} on the kernel command line"
                );
                power_off();
            };
            let control =
                match VsockStream::connect_retrying(VMADDR_CID_HOST, port, Duration::from_secs(30))
                {
                    Ok(control) => control,
                    Err(e) => {
                        eprintln!("overdrive-guest-peer: dialling the host on {port}: {e}");
                        power_off();
                    }
                };
            eprintln!("overdrive-guest-peer: control connection to the host on {port}");
            let state = Arc::new(State::default());
            let mut pending = Vec::new();
            let mut buf = vec![0u8; 65_536];
            loop {
                let line = loop {
                    if let Some(at) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=at).collect();
                        break String::from_utf8_lossy(&line[..at]).into_owned();
                    }
                    match (&control).read(&mut buf) {
                        Ok(0) | Err(_) => power_off(),
                        Ok(n) => pending.extend_from_slice(&buf[..n]),
                    }
                };
                let reply = match line.parse::<PeerCommand>() {
                    Err(detail) => failure(format!("unparsed command {line:?}: {detail}")),
                    Ok(PeerCommand::CrashKernel) => {
                        // SAFETY: `sync` has no preconditions.
                        unsafe { libc::sync() };
                        let _ = std::fs::write("/proc/sysrq-trigger", b"c");
                        failure("the kernel did not crash")
                    }
                    Ok(PeerCommand::PowerOff) => {
                        let _ = (&control).write_all(format!("{}\n", PeerReply::Ok).as_bytes());
                        power_off();
                    }
                    Ok(command) => execute(&state, command),
                };
                if (&control).write_all(format!("{reply}\n").as_bytes()).is_err() {
                    power_off();
                }
            }
        }

        const fn millis(ms: u64) -> Duration {
            Duration::from_millis(ms)
        }

        const fn side(side: PairSide) -> PeerSide {
            match side {
                PairSide::A => PeerSide::A,
                PairSide::B => PeerSide::B,
            }
        }

        const fn peer_end(end: &ForwardEnd) -> PeerEnd {
            let outcome = match end.outcome {
                ForwardOutcome::Completed => PeerOutcome::Completed,
                ForwardOutcome::GuestHalfClose => PeerOutcome::GuestHalfClose,
                ForwardOutcome::Aborted { cause } => match cause {
                    AbortCause::PeerAbort { side: s } => PeerOutcome::PeerAbort(side(s)),
                    AbortCause::SocketError { side: s, errno } => {
                        PeerOutcome::SocketError(side(s), errno.get())
                    }
                    AbortCause::FrameInvalid { side: s } => PeerOutcome::FrameInvalid(side(s)),
                    AbortCause::Detached => PeerOutcome::Detached,
                    AbortCause::Unclassified { side: s } => PeerOutcome::Unclassified(side(s)),
                },
            };
            PeerEnd { id: end.id.get(), outcome, moved_ab: end.moved_ab, moved_ba: end.moved_ba }
        }

        fn forwarder(resource: &Resource) -> Result<&KernelFlowForwarder, PeerReply> {
            match resource {
                Resource::Forwarder(ctx) => Ok(ctx),
                _ => Err(failure("not a forwarder context")),
            }
        }

        fn write_all(resource: &Resource, bytes: &[u8]) -> io::Result<()> {
            match resource {
                Resource::Vsock(s) => (&*s).write_all(bytes),
                Resource::Tcp(s) => (&*s).write_all(bytes),
                _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
            }
        }

        fn read_once(resource: &Resource, buf: &mut [u8]) -> io::Result<usize> {
            match resource {
                Resource::Vsock(s) => (&*s).read(buf),
                Resource::Tcp(s) => (&*s).read(buf),
                _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
            }
        }

        fn readable(resource: &Resource, timeout: Duration) -> io::Result<bool> {
            wait_ready(resource.as_fd(), libc::POLLIN | libc::POLLRDHUP, timeout)
        }

        fn set_abortive_linger(stream: &TcpStream) -> io::Result<()> {
            let linger = libc::linger { l_onoff: 1, l_linger: 0 };
            #[allow(
                clippy::cast_possible_truncation,
                reason = "sizeof(struct linger) fits socklen_t"
            )]
            let len = std::mem::size_of::<libc::linger>() as libc::socklen_t;
            // SAFETY: `linger` is a valid option value of the passed length.
            let ret = unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    (&raw const linger).cast(),
                    len,
                )
            };
            if ret == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
        }

        #[allow(clippy::too_many_lines, reason = "one arm per protocol verb")]
        fn execute(state: &Arc<State>, command: PeerCommand) -> PeerReply {
            let result = (|| -> Result<PeerReply, PeerReply> {
                Ok(match command {
                    PeerCommand::VsockConnect { handle, cid, port, source_port } => {
                        match VsockStream::connect(cid, port, source_port) {
                            Ok(stream) => state.put(handle, Resource::Vsock(stream)),
                            Err(e) => err(format!("vsock connect ({cid}, {port})"), &e),
                        }
                    }
                    PeerCommand::VsockListen { handle, port } => {
                        match VsockListener::bind(VMADDR_CID_ANY, port) {
                            Ok(listener) => state.put(handle, Resource::VsockListener(listener)),
                            Err(e) => err(format!("vsock listen {port}"), &e),
                        }
                    }
                    PeerCommand::VsockAccept { handle, listener, timeout_ms } => {
                        let resource = state.get(&listener)?;
                        let Resource::VsockListener(l) = &*resource else {
                            return Err(failure("not a vsock listener"));
                        };
                        match l.accept_within(millis(timeout_ms)) {
                            Ok(Some((stream, peer))) => {
                                match state.put(handle, Resource::Vsock(stream)) {
                                    PeerReply::Ok => {
                                        PeerReply::Accepted { cid: peer.cid, port: peer.port }
                                    }
                                    other => other,
                                }
                            }
                            Ok(None) => PeerReply::Timeout,
                            Err(e) => err("vsock accept", &e),
                        }
                    }
                    PeerCommand::TcpListen { handle, addr } => match TcpListener::bind(addr) {
                        Ok(listener) => {
                            let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
                            match state.put(handle, Resource::TcpListener(listener)) {
                                PeerReply::Ok => PeerReply::Value(u64::from(port)),
                                other => other,
                            }
                        }
                        Err(e) => err(format!("tcp listen {addr}"), &e),
                    },
                    PeerCommand::TcpConnect { handle, addr } => match TcpStream::connect(addr) {
                        Ok(stream) => state.put(handle, Resource::Tcp(stream)),
                        Err(e) => err(format!("tcp connect {addr}"), &e),
                    },
                    PeerCommand::TcpAccept { handle, listener, timeout_ms } => {
                        let resource = state.get(&listener)?;
                        let Resource::TcpListener(l) = &*resource else {
                            return Err(failure("not a tcp listener"));
                        };
                        match wait_ready(l.as_fd(), libc::POLLIN, millis(timeout_ms)) {
                            Ok(false) => PeerReply::Timeout,
                            Ok(true) => match l.accept() {
                                Ok((stream, _)) => state.put(handle, Resource::Tcp(stream)),
                                Err(e) => err("tcp accept", &e),
                            },
                            Err(e) => err("poll", &e),
                        }
                    }
                    PeerCommand::Write { handle, bytes } => {
                        let resource = state.get(&handle)?;
                        match write_all(&resource, &bytes) {
                            Ok(()) => PeerReply::Ok,
                            Err(e) => err("write", &e),
                        }
                    }
                    PeerCommand::Read { handle, max, timeout_ms } => {
                        let resource = state.get(&handle)?;
                        match readable(&resource, millis(timeout_ms)) {
                            Ok(false) => PeerReply::Timeout,
                            Ok(true) => {
                                let mut buf =
                                    vec![0u8; usize::try_from(max.max(1)).unwrap_or(65_536)];
                                match read_once(&resource, &mut buf) {
                                    Ok(0) => PeerReply::Eof,
                                    Ok(n) => PeerReply::Data(buf[..n].to_vec()),
                                    Err(e) => err("read", &e),
                                }
                            }
                            Err(e) => err("poll", &e),
                        }
                    }
                    PeerCommand::Pump { handle, tag, len } => {
                        let resource = state.get(&handle)?;
                        let mut stream_gen = StreamGen::new(&tag, Direction::GuestToPeer);
                        let mut chunk = vec![0u8; STREAM_CHUNK];
                        let mut written = 0u64;
                        while written < len {
                            let n = usize::try_from((len - written).min(STREAM_CHUNK as u64))
                                .unwrap_or(STREAM_CHUNK);
                            stream_gen.fill(&mut chunk[..n]);
                            if let Err(e) = write_all(&resource, &chunk[..n]) {
                                return Ok(err(format!("pump after {written} bytes"), &e));
                            }
                            written += n as u64;
                        }
                        PeerReply::Value(written)
                    }
                    PeerCommand::Drain { handle, tag, timeout_ms } => {
                        let resource = state.get(&handle)?;
                        let mut check = StreamCheck::new(&tag, Direction::PeerToGuest);
                        let mut buf = vec![0u8; STREAM_CHUNK];
                        let end = loop {
                            match readable(&resource, millis(timeout_ms)) {
                                Ok(false) => break DrainEnd::Timeout,
                                Ok(true) => {}
                                Err(e) => break DrainEnd::Error(e.raw_os_error().unwrap_or(-1)),
                            }
                            match read_once(&resource, &mut buf) {
                                Ok(0) => break DrainEnd::Eof,
                                Ok(n) => {
                                    if let Err(at) = check.check(&buf[..n]) {
                                        break DrainEnd::Mismatch(at);
                                    }
                                }
                                Err(e) => match TerminalEvent::from_read_error(&e) {
                                    TerminalEvent::Reset => break DrainEnd::Reset,
                                    TerminalEvent::Error(code) => break DrainEnd::Error(code),
                                    TerminalEvent::Eof => break DrainEnd::Eof,
                                },
                            }
                        };
                        PeerReply::Drained { len: check.verified(), end }
                    }
                    PeerCommand::Shutdown { handle, how } => {
                        let resource = state.get(&handle)?;
                        let shut: io::Result<()> = match &*resource {
                            Resource::Vsock(s) => s.shutdown(how),
                            Resource::Tcp(s) => s.shutdown(how),
                            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
                        };
                        match shut {
                            Ok(()) => PeerReply::Ok,
                            Err(e) => err("shutdown", &e),
                        }
                    }
                    PeerCommand::Close { handle } | PeerCommand::ForwarderClose { ctx: handle } => {
                        match state.take(&handle) {
                            Some(_) => PeerReply::Ok,
                            None => failure(format!("no resource {handle:?}")),
                        }
                    }
                    PeerCommand::Reset { handle } => {
                        let resource = state
                            .take(&handle)
                            .ok_or_else(|| failure(format!("no resource {handle:?}")))?;
                        if let Resource::Tcp(stream) = &*resource
                            && let Err(e) = set_abortive_linger(stream)
                        {
                            return Ok(err("SO_LINGER", &e));
                        }
                        drop(resource);
                        PeerReply::Ok
                    }
                    PeerCommand::ForwarderOpen { ctx } => match KernelFlowForwarder::open() {
                        Ok(forwarder) => state.put(ctx, Resource::Forwarder(forwarder)),
                        Err(e) => failure(format!("KernelFlowForwarder::open: {e}")),
                    },
                    PeerCommand::ForwarderAttachStream { ctx, vsock, tcp } => {
                        let context = state.get(&ctx)?;
                        let vsock = state.get(&vsock)?;
                        let tcp = state.get(&tcp)?;
                        let context = forwarder(&context)?;
                        let a: &dyn AsFd = &*vsock;
                        let b: &dyn AsFd = &*tcp;
                        match context.attach(a, b, PairMode::Stream) {
                            Ok(id) => PeerReply::Forward(id.get()),
                            Err(e) => PeerReply::Refused(format!("{e:?}")),
                        }
                    }
                    PeerCommand::ForwarderWatch { ctx, vsock } => {
                        let context = state.get(&ctx)?;
                        let vsock = state.get(&vsock)?;
                        let context = forwarder(&context)?;
                        let a: &dyn AsFd = &*vsock;
                        match context.watch(a) {
                            Ok(id) => PeerReply::Forward(id.get()),
                            Err(e) => PeerReply::Refused(format!("{e:?}")),
                        }
                    }
                    PeerCommand::ForwarderAbort { ctx, id } => {
                        let context = state.get(&ctx)?;
                        let context = forwarder(&context)?;
                        let id = ForwardId::new(id)
                            .map_err(|e| failure(format!("forward id {id}: {e:?}")))?;
                        // `VALUE 1`: the id was live and its record is queued;
                        // `VALUE 0`: it had already ended.
                        match context.abort(id) {
                            Ok(AbortResult::Aborted) => PeerReply::Value(1),
                            Ok(AbortResult::AlreadyEnded) => PeerReply::Value(0),
                            Err(e) => failure(format!("abort: {e}")),
                        }
                    }
                    PeerCommand::ForwarderTakeEnd { ctx, timeout_ms } => {
                        let resource = state.get(&ctx)?;
                        let context = forwarder(&resource)?;
                        let deadline = Instant::now() + millis(timeout_ms);
                        loop {
                            match context.take_end() {
                                Ok(Some(end)) => break PeerReply::End(peer_end(&end)),
                                Ok(None) => {}
                                Err(e) => break failure(format!("take_end: {e}")),
                            }
                            let left = deadline.saturating_duration_since(Instant::now());
                            if left.is_zero() {
                                break PeerReply::NoEnd;
                            }
                            if let Err(e) = wait_ready(context.readiness_fd(), libc::POLLIN, left) {
                                break err("poll the forwarder context", &e);
                            }
                        }
                    }
                    PeerCommand::Spawn { task, command } => {
                        let worker = Arc::clone(state);
                        let handle = thread::spawn(move || execute(&worker, *command));
                        state
                            .tasks
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(task, handle);
                        PeerReply::Ok
                    }
                    PeerCommand::Join { task, timeout_ms } => {
                        let deadline = Instant::now() + millis(timeout_ms);
                        loop {
                            let finished = state
                                .tasks
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .get(&task)
                                .map(JoinHandle::is_finished);
                            match finished {
                                None => break failure(format!("no task {task:?}")),
                                Some(true) => {
                                    let handle = state
                                        .tasks
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .remove(&task);
                                    break handle.map_or_else(
                                        || failure(format!("no task {task:?}")),
                                        |h| {
                                            h.join()
                                                .unwrap_or_else(|_| failure("the task panicked"))
                                        },
                                    );
                                }
                                Some(false) if Instant::now() >= deadline => {
                                    break PeerReply::Pending;
                                }
                                Some(false) => thread::sleep(Duration::from_millis(10)),
                            }
                        }
                    }
                    PeerCommand::Exec { argv } => {
                        let Some((program, args)) = argv.split_first() else {
                            return Err(failure("empty argv"));
                        };
                        match Command::new(program).args(args).status() {
                            Ok(status) => {
                                use std::os::unix::process::ExitStatusExt as _;
                                PeerReply::Exit(
                                    status
                                        .code()
                                        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
                                )
                            }
                            Err(e) => err(format!("exec {program}"), &e),
                        }
                    }
                    PeerCommand::CrashKernel | PeerCommand::PowerOff => {
                        failure("handled by the control loop")
                    }
                })
            })();
            result.unwrap_or_else(|reply| reply)
        }
    }
}
