//! Bounded direct stock-transport reproduction. Both sides are test endpoints;
//! no proxy-leg payload is read or relayed and no BPF program is loaded.
mod peer;
use peer::Peer;
use serde_json::json;
use std::{
    fs,
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    thread,
    time::{Duration, Instant},
};
#[repr(C)]
struct Addr {
    family: u16,
    reserved: u16,
    port: u32,
    cid: u32,
    flags: u8,
    pad: [u8; 3],
}
fn event(v: serde_json::Value) {
    println!("{v}");
    io::stdout().flush().unwrap()
}
fn listener() -> OwnedFd {
    unsafe {
        let f = OwnedFd::from_raw_fd(libc::socket(libc::AF_VSOCK, libc::SOCK_SEQPACKET, 0));
        let a = Addr {
            family: libc::AF_VSOCK as u16,
            reserved: 0,
            port: 29500,
            cid: 2,
            flags: 0,
            pad: [0; 3],
        };
        assert_eq!(libc::bind(f.as_raw_fd(), &a as *const _ as *const _, 16), 0);
        assert_eq!(libc::listen(f.as_raw_fd(), 16), 0);
        f
    }
}
fn frame(nonce: u64) -> Vec<u8> {
    let mut b = b"ZUD1".to_vec();
    b.extend(1431u32.to_be_bytes());
    let mut payload: Vec<u8> = (0..1431).map(|j| ((j * 31 + nonce as usize) % 251) as u8).collect();
    payload[..8].copy_from_slice(&nonce.to_le_bytes());
    b.extend(payload);
    b
}
fn send(fd: i32, b: &[u8]) -> (isize, i32) {
    let n = unsafe {
        libc::send(fd, b.as_ptr() as *const _, b.len(), libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL)
    };
    let errno = if n < 0 { io::Error::last_os_error().raw_os_error().unwrap() } else { 0 };
    (n, errno)
}
fn receive(p: &mut Peer) -> (u32, Vec<u8>) {
    loop {
        let (op, flags, b) = p.rx();
        if op == 5 {
            return (flags, b);
        }
        assert_eq!(op, 6)
    }
}
fn main() {
    let start = Instant::now();
    let vl = listener();
    let mut p = Peer::new(320777, 7000, 2);
    p.connect();
    let fd = unsafe {
        OwnedFd::from_raw_fd(libc::accept(
            vl.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ))
    };
    assert!(fd.as_raw_fd() >= 0);
    event(
        json!({"event":"direct_seqpacket_begin","real_vhost_fd":p.fd.as_raw_fd(),"cid":p.cid,"memory":format!("{:p}",p.mem),"bpf_loaded":false,"userspace_endpoint_producer_not_relay":true,"advertised_peer_credit":65536,"encoded_message_len":1439}),
    );
    for i in 0..45 {
        let b = frame(i);
        let (n, e) = send(fd.as_raw_fd(), &b);
        assert_eq!((n, e), (1439, 0));
    }
    let retry = frame(45);
    let first = send(fd.as_raw_fd(), &retry);
    event(
        json!({"event":"direct_nonblocking_send_at_credit_remainder","return":first.0,"errno":first.1,"prior_bytes_sent":45*1439,"remaining_advertised_credit":781}),
    );
    assert_eq!(first, (-1, libc::EAGAIN));
    for i in 0..45 {
        let (flags, b) = receive(&mut p);
        assert_eq!(b, frame(i));
        assert_ne!(flags & 1, 0);
    }
    let (flags, partial) = receive(&mut p);
    assert_eq!(partial, &retry[..781]);
    event(
        json!({"event":"direct_partial_despite_eagain","virtio_flags":flags,"wire_len":partial.len(),"nonce":45,"matches_prefix_of_retried_message":true,"partial_hex":partial.iter().map(|b|format!("{b:02x}")).collect::<String>()}),
    );
    thread::sleep(Duration::from_millis(50));
    let second = send(fd.as_raw_fd(), &retry);
    assert_eq!(second, (1439, 0));
    let (flags, full) = receive(&mut p);
    assert_eq!(full, retry);
    assert_ne!(flags & 1, 0);
    event(
        json!({"event":"direct_full_retry_duplicate_prefix_proven","return":second.0,"errno":second.1,"virtio_flags":flags,"full_wire_len":full.len(),"partial_plus_full_bytes":partial.len()+full.len(),"bpf_loaded":false,"diagnostic_conclusion":"stock nonblocking SEQPACKET send returned EAGAIN after emitting a partial frame; retrying emits that prefix again"}),
    );
    p.tx(3, 0, &[]);
    drop(fd);
    drop(p);
    drop(vl);
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let m = fs::read_to_string("/proc/modules").unwrap();
        let n = m
            .lines()
            .find(|x| x.starts_with("vhost_vsock "))
            .unwrap()
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        if n == 0 {
            event(
                json!({"event":"transport_drain_live_kernel","usage":0,"confirmed_zero":true,"modules":m}),
            );
            break;
        }
        assert!(Instant::now() < until, "direct endpoint transport drain timeout");
        thread::sleep(Duration::from_millis(20));
    }
    event(
        json!({"event":"complete","elapsed_s":start.elapsed().as_secs_f64(),"direct_reproduction_passed":true}),
    );
}
