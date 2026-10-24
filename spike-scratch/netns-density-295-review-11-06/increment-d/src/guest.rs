use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

fn new_socket() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("ordinary UDP bind");
    socket.set_read_timeout(Some(Duration::from_secs(2))).expect("bounded receive");
    socket
}

fn main() {
    let destination = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 99), 7001);
    println!("OBSERVED_RELEASE_CONTROL ordinary send_to, exact echo, then drop; production init PID 1");
    for sequence in 0..1100u32 {
        let socket = new_socket();
        let payload = sequence.to_be_bytes();
        assert_eq!(socket.send_to(&payload, destination).expect("control send"), payload.len());
        let mut buffer = [0u8; 128];
        let (length, _) = socket.recv_from(&mut buffer).expect("control echo before close");
        assert_eq!(&buffer[..length], &payload);
        drop(socket);
    }
    println!("SHORT_LIVED_CONTROL echoed=1100 live_application_sockets=0");
    std::thread::sleep(Duration::from_secs(3));
    let probe = new_socket();
    let payload = b"after-all-applications-closed";
    assert_eq!(probe.send_to(payload, destination).expect("post-audit control send"), payload.len());
    let mut buffer = [0u8; 128];
    let (length, peer) = probe.recv_from(&mut buffer).expect("post-audit control echo");
    assert_eq!(&buffer[..length], payload);
    println!("POST_AUDIT_RESULT=ECHOED peer={peer}");
}
