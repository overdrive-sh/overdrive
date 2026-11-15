// Throwaway guest application (std only, built static-musl by the harness).
//   probe echo <port>                         TCP "pong:<line>" + UDP "echo:<data>" on 0.0.0.0:<port>
//   probe client <vip> <port> <backend-addr>  TCP/UDP to the VIP, UDP to the backend directly
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::thread;
use std::time::Duration;

fn udp_try(label: &str, target: SocketAddr, tries: u32) -> bool {
    for n in 1..=tries {
        let s = match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(target).map(|()| s)) {
            Ok(s) => s,
            Err(e) => { println!("probe: {label} try={n} socket/connect error: {e}"); thread::sleep(Duration::from_secs(2)); continue; }
        };
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        if let Err(e) = s.send(b"udp-ping") { println!("probe: {label} try={n} send error: {e}"); continue; }
        let mut buf = [0u8; 256];
        match s.recv(&mut buf) {
            Ok(k) => { println!("probe: {label} try={n} REPLY {:?}", String::from_utf8_lossy(&buf[..k])); return true; }
            Err(e) => println!("probe: {label} try={n} NO-REPLY kind={:?} err={e}", e.kind()),
        }
    }
    false
}

fn tcp_try(label: &str, target: SocketAddr, tries: u32) -> bool {
    for n in 1..=tries {
        match TcpStream::connect_timeout(&target, Duration::from_secs(5)) {
            Ok(mut s) => {
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let _ = s.write_all(b"tcp-ping\n");
                let mut buf = [0u8; 256];
                match s.read(&mut buf) {
                    Ok(k) if k > 0 => { println!("probe: {label} try={n} REPLY {:?}", String::from_utf8_lossy(&buf[..k])); return true; }
                    other => println!("probe: {label} try={n} read result {other:?}"),
                }
            }
            Err(e) => println!("probe: {label} try={n} connect error: {e}"),
        }
        thread::sleep(Duration::from_secs(2));
    }
    false
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("echo") => {
            let port: u16 = a[2].parse().unwrap();
            let udp = UdpSocket::bind(("0.0.0.0", port)).unwrap();
            let tcp = TcpListener::bind(("0.0.0.0", port)).unwrap();
            println!("probe-echo: listening udp+tcp {port}");
            thread::spawn(move || {
                for c in tcp.incoming().flatten() {
                    thread::spawn(move || {
                        let mut c = c;
                        let mut b = [0u8; 256];
                        if let Ok(k) = c.read(&mut b) {
                            let mut r = b"pong:".to_vec(); r.extend_from_slice(&b[..k]);
                            let _ = c.write_all(&r);
                            println!("probe-echo: tcp request served");
                        }
                    });
                }
            });
            let mut b = [0u8; 2048];
            loop {
                if let Ok((k, from)) = udp.recv_from(&mut b) {
                    println!("probe-echo: UDP datagram len={k} from={from}");
                    let mut r = b"echo:".to_vec(); r.extend_from_slice(&b[..k]);
                    let _ = udp.send_to(&r, from);
                }
            }
        }
        Some("client") => {
            let vip: std::net::Ipv4Addr = a[2].parse().unwrap();
            let port: u16 = a[3].parse().unwrap();
            let backend: SocketAddr = a[4].parse().unwrap();
            let vip_addr = SocketAddr::from((vip, port));
            println!("probe: start vip={vip_addr} backend={backend}");
            thread::sleep(Duration::from_secs(12)); // let the SVID converge, as the skeleton does
            println!("probe: vip-tcp {}", if tcp_try("vip-tcp", vip_addr, 15) { "ok" } else { "FAILED" });
            println!("probe: backend-udp-direct {}", if udp_try("backend-udp", backend, 2) { "ok" } else { "FAILED" });
            println!("probe: vip-udp-phase-1 {}", if udp_try("vip-udp", vip_addr, 4) { "ok" } else { "FAILED" });
            println!("probe: dump-point");
            thread::sleep(Duration::from_secs(8));
            println!("probe: vip-udp-phase-2 {}", if udp_try("vip-udp", vip_addr, 4) { "ok" } else { "FAILED" });
            println!("probe: done");
            loop { thread::sleep(Duration::from_secs(3600)); }
        }
        _ => { eprintln!("usage"); std::process::exit(2); }
    }
}
