//! The "workload": its FIRST syscalls (after libc start) are socket()+bind()+listen(); then a UDP bind; then exit.
//! argv: <tcp_port> <udp_port>
use std::mem;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let tcp: u16 = a[1].parse().unwrap();
    let udp: u16 = a[2].parse().unwrap();
    unsafe {
        let s = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        let mut sa: libc::sockaddr_in = mem::zeroed();
        sa.sin_family = libc::AF_INET as _;
        sa.sin_port = tcp.to_be();
        libc::bind(s, &sa as *const _ as *const _, 16);
        libc::listen(s, 16);
        let u = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        sa.sin_port = udp.to_be();
        libc::bind(u, &sa as *const _ as *const _, 16);
    }
}
