use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

fn new_socket() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("ordinary UDP bind");
    socket.set_read_timeout(Some(Duration::from_secs(2))).expect("bounded receive");
    socket
}

fn main() {
    let destination = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 99), 7001);
    println!("IMMEDIATE_RELEASE_PROBE ordinary send_to then drop, production init PID 1");
    let mut admitted = 0usize;
    let mut refused = 0usize;
    for sequence in 0..1100u32 {
        let socket = new_socket();
        match socket.send_to(&sequence.to_be_bytes(), destination) {
            Ok(4) => admitted += 1,
            Err(error) if error.raw_os_error() == Some(1) => refused += 1,
            other => panic!("unexpected send at {sequence}: {other:?}"),
        }
        drop(socket);
    }
    println!("SHORT_LIVED_SENDS admitted={admitted} refused={refused} live_application_sockets=0");
    println!("WAIT_FOR_AUDIT seconds=3 approved_period_seconds=1");
    std::thread::sleep(Duration::from_secs(3));
    let probe = new_socket();
    let payload = b"after-all-applications-closed";
    match probe.send_to(payload, destination) {
        Ok(written) => {
            assert_eq!(written, payload.len());
            let mut buffer = [0u8; 128];
            match probe.recv_from(&mut buffer) {
                Ok((length, peer)) if &buffer[..length] == payload => {
                    println!("POST_AUDIT_RESULT=ECHOED peer={peer}");
                }
                other => {
                    println!("POST_AUDIT_RESULT=ADMITTED_WITHOUT_ECHO observation={other:?}");
                    std::process::exit(3);
                }
            }
        }
        Err(error) if error.raw_os_error() == Some(1) => {
            println!("POST_AUDIT_RESULT=EXHAUSTED errno=EPERM live_prior_application_sockets=0");
            std::process::exit(2);
        }
        other => panic!("unexpected post-audit send: {other:?}"),
    }
}
