//! V-18 test applications (throwaway spike, netns-density-295).
//!   vfwd v18-send  guest application: sends frame-shaped / near-frame / random payloads of chosen
//!                  sizes to a peer, one datagram per exchange, and checks the peer's ack
//!                  (be32 length + SHA-256 of the datagram the peer RECEIVED).
//!   vfwd v18-peer  the off-host peer: logs {len, sha256, head} of every received datagram and acks.
//! Both know nothing about vsock or frames: they are ordinary AF_INET UDP applications.
use serde_json::json;
use std::net::{SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAGIC: [u8; 4] = *b"OVD1";

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}
fn flag(args: &[String], k: &str) -> bool {
    args.iter().any(|a| a == k)
}

// ------------------------------------------------------------------ SHA-256 (no crate available offline)
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let t2 = s0.wrapping_add(maj);
            a = [t1.wrapping_add(t2), a[0], a[1], a[2], a[3].wrapping_add(t1), a[4], a[5], a[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn selftest() {
    let abc = hex(&sha256(b"abc"));
    let empty = hex(&sha256(b""));
    let long = hex(&sha256(&vec![b'a'; 1_000_000]));
    println!(
        "{}",
        json!({"ev":"sha_selftest",
          "abc": abc == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
          "empty": empty == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          "million_a": long == "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"})
    );
}

// ------------------------------------------------------------------ payloads
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            v.extend_from_slice(&self.next().to_le_bytes());
        }
        v.truncate(n);
        v
    }
}

fn hdr(declared: u32) -> [u8; 8] {
    let mut h = [0u8; 8];
    h[..4].copy_from_slice(&MAGIC);
    h[4..].copy_from_slice(&declared.to_be_bytes());
    h
}

/// The escape-rule predicate: the payload itself parses as a valid datagram frame.
pub fn shaped(p: &[u8]) -> bool {
    p.len() >= 8 && p[..4] == MAGIC && u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize == p.len() - 8
}

/// Build a payload of exactly `len` bytes of the named kind, or None when the kind does not fit.
fn payload(kind: &str, len: usize, seed: u64) -> Option<Vec<u8>> {
    let mut r = Rng(seed | 1);
    let l32 = len as u32;
    let with_hdr = |declared: u32, min: usize| -> Option<Vec<u8>> {
        if len < min {
            return None;
        }
        let mut r = Rng(seed | 1);
        let mut p = hdr(declared).to_vec();
        p.extend(r.bytes(len - 8));
        Some(p)
    };
    match kind {
        "rand" => {
            let mut p = r.bytes(len);
            if len > 0 && p[0] == b'O' {
                p[0] = b'P';
            }
            Some(p)
        }
        "zero" => Some(vec![0; len]),
        "ones" => Some(vec![0xff; len]),
        // frame-shaped: the payload itself is a valid frame (escape rule)
        "frame" => with_hdr(l32.wrapping_sub(8), 8),
        "frame2" => {
            if len < 16 {
                return None;
            }
            let mut p = hdr(l32 - 8).to_vec();
            p.extend_from_slice(&hdr(l32 - 16));
            p.extend(r.bytes(len - 16));
            Some(p)
        }
        "frame3" => {
            if len < 24 {
                return None;
            }
            let mut p = hdr(l32 - 8).to_vec();
            p.extend_from_slice(&hdr(l32 - 16));
            p.extend_from_slice(&hdr(l32 - 24));
            p.extend(r.bytes(len - 24));
            Some(p)
        }
        // every 8-byte block is a frame header (declared = bytes remaining after it): the content at
        // every fragment boundary looks like a frame. first block valid => frame-shaped.
        "frame_chain" | "nf_chain" => {
            if len < 8 {
                return None;
            }
            let mut p = Vec::with_capacity(len);
            let mut off = 0;
            while off + 8 <= len {
                let mut d = (len - off - 8) as u32;
                if off == 0 && kind == "nf_chain" {
                    d = d.wrapping_add(1); // first block wrong => not frame-shaped
                }
                p.extend_from_slice(&hdr(d));
                off += 8;
            }
            p.extend(r.bytes(len - p.len()));
            Some(p)
        }
        // near misses: magic present, declared length wrong => NOT frame-shaped
        "decl_len" => with_hdr(l32, 8),
        "decl_m7" => with_hdr(l32.wrapping_sub(7), 8),
        "decl_m9" => with_hdr(l32.wrapping_sub(9), 8),
        "decl_m16" => with_hdr(l32.wrapping_sub(16), 8),
        "decl_zero" => with_hdr(0, 8),
        "decl_max" => with_hdr(u32::MAX, 8),
        "decl_59000" => with_hdr(59000, 8),
        // declared = what the first fragment alone would carry (fragment-confusion)
        "decl_1464" => with_hdr(1464, 8),
        "decl_1472" => with_hdr(1472, 8),
        "decl_1480" => with_hdr(1480, 8),
        // declared = frame-shaped for the FRAMED size (len+8-8 = len is decl_len); the TC-side confusion
        "decl_p8" => with_hdr(l32.wrapping_add(8), 8),
        "badmagic" => {
            if len < 8 {
                return None;
            }
            let mut p = Rng(seed | 1).bytes(len);
            p[..4].copy_from_slice(b"OVD2");
            p[4..8].copy_from_slice(&(l32 - 8).to_be_bytes());
            Some(p)
        }
        "badmagic_case" => {
            if len < 8 {
                return None;
            }
            let mut p = Rng(seed | 1).bytes(len);
            p[..4].copy_from_slice(b"ovd1");
            p[4..8].copy_from_slice(&(l32 - 8).to_be_bytes());
            Some(p)
        }
        // magic only, random length field
        "magic_only" => {
            if len < 8 {
                return None;
            }
            let mut p = Rng(seed | 1).bytes(len);
            p[..4].copy_from_slice(&MAGIC);
            if shaped(&p) {
                p[7] ^= 1;
            }
            Some(p)
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ sender
struct Case {
    kind: String,
    len: usize,
    seed: u64,
}

fn exchange(s: &UdpSocket, dst: SocketAddrV4, connected: bool, c: &Case, thread: usize, seq: bool, out: &Mutex<()>) -> bool {
    let Some(p) = payload(&c.kind, c.len, c.seed) else { return true };
    let digest = sha256(&p);
    let sent = if connected { s.send(&p) } else { s.send_to(&p, dst) };
    let mut rec = json!({"ev":"v18","t":thread,"kind":c.kind,"len":c.len,"conn":connected,"seq":seq,"seed":c.seed,
        "shaped":shaped(&p),"sha":hex(&digest),"head":hex(&p[..p.len().min(8)])});
    let mut ok = false;
    match sent {
        Err(e) => rec["err"] = json!(format!("send {e}")),
        Ok(_) => {
            let mut buf = [0u8; 2048];
            loop {
                match s.recv_from(&mut buf) {
                    Ok((m, from)) => {
                        if from != SocketAddr::V4(dst) {
                            continue;
                        }
                        rec["ack_len_bytes"] = json!(m);
                        if m == 36 {
                            let rl = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                            rec["ack_len"] = json!(rl);
                            rec["ack_sha_match"] = json!(buf[4..36] == digest);
                            ok = rl == p.len() && buf[4..36] == digest;
                        }
                        break;
                    }
                    Err(e) => {
                        rec["err"] = json!(format!("recv {e}"));
                        break;
                    }
                }
            }
        }
    }
    rec["ok"] = json!(ok);
    let _g = out.lock().unwrap();
    println!("{rec}");
    ok
}

pub fn send(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let kinds: Vec<String> = arg(args, "--kinds").unwrap().split(',').map(|s| s.to_string()).collect();
    let sizes: Vec<usize> = arg(args, "--sizes").unwrap().split(',').map(|s| s.parse().unwrap()).collect();
    let reps: usize = arg(args, "--reps").map(|s| s.parse().unwrap()).unwrap_or(1);
    let seed: u64 = arg(args, "--seed").map(|s| s.parse().unwrap()).unwrap_or(1);
    let threads: usize = arg(args, "--threads").map(|s| s.parse().unwrap()).unwrap_or(1);
    let random: usize = arg(args, "--random").map(|s| s.parse().unwrap()).unwrap_or(0);
    let tmo: u64 = arg(args, "--timeout-ms").map(|s| s.parse().unwrap()).unwrap_or(2000);
    let hold: u64 = arg(args, "--hold-ms").map(|s| s.parse().unwrap()).unwrap_or(0);
    let connected = flag(args, "--connect");
    let seq = flag(args, "--seq");
    let out = Arc::new(Mutex::new(()));
    let t0 = Instant::now();
    let mut cases: Vec<Case> = vec![];
    if random == 0 {
        let mut n = 0u64;
        for k in &kinds {
            for &l in &sizes {
                for rep in 0..reps {
                    n += 1;
                    cases.push(Case { kind: k.clone(), len: l, seed: seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(n * 7919 + rep as u64) });
                }
            }
        }
    }
    let cases = Arc::new(cases);
    let kinds = Arc::new(kinds);
    let sizes = Arc::new(sizes);
    let mut hs = vec![];
    let (okc, failc) = (Arc::new(Mutex::new(0u64)), Arc::new(Mutex::new(0u64)));
    for t in 0..threads {
        let (cases, kinds, sizes, out, okc, failc) = (cases.clone(), kinds.clone(), sizes.clone(), out.clone(), okc.clone(), failc.clone());
        hs.push(std::thread::spawn(move || {
            let mk = || {
                let s = UdpSocket::bind("0.0.0.0:0").unwrap();
                s.set_read_timeout(Some(Duration::from_millis(tmo))).unwrap();
                if connected {
                    s.connect(dst).unwrap();
                }
                s
            };
            let mut sock = mk();
            let mut run = |c: &Case, sock: &mut UdpSocket| {
                if !seq {
                    *sock = mk();
                }
                if exchange(sock, dst, connected, c, t, seq, &out) {
                    *okc.lock().unwrap() += 1
                } else {
                    *failc.lock().unwrap() += 1
                }
            };
            if random > 0 {
                let mut r = Rng(seed.wrapping_add(t as u64 * 0x1234567) | 1);
                for i in 0..random {
                    let k = kinds[(r.next() % kinds.len() as u64) as usize].clone();
                    let l = sizes[(r.next() % sizes.len() as u64) as usize];
                    let c = Case { kind: k, len: l, seed: r.next() ^ i as u64 };
                    run(&c, &mut sock);
                }
            } else {
                for (i, c) in cases.iter().enumerate() {
                    if i % threads == t {
                        run(c, &mut sock);
                    }
                }
            }
            sock
        }));
    }
    let socks: Vec<UdpSocket> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    println!(
        "{}",
        json!({"ev":"v18_done","dst":dst.to_string(),"ok":*okc.lock().unwrap(),"failed":*failc.lock().unwrap(),"threads":threads,"connected":connected,"seq":seq,"s":t0.elapsed().as_secs_f64(),
               "local_ports": socks.iter().map(|s| s.local_addr().map(|a| a.port()).unwrap_or(0)).collect::<Vec<_>>()})
    );
    if hold > 0 {
        std::thread::sleep(Duration::from_millis(hold));
    }
    drop(socks);
}

// ------------------------------------------------------------------ peer
pub fn peer(args: &[String]) {
    let bind = arg(args, "--bind").unwrap();
    let s = UdpSocket::bind(&bind).unwrap();
    let rb = 16 << 20;
    unsafe {
        use std::os::fd::AsRawFd;
        let v: libc::c_int = rb;
        libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, &v as *const _ as *const _, 4);
    }
    println!("{}", json!({"ev":"v18_peer_ready","bind":bind}));
    let mut buf = vec![0u8; 65536];
    let mut n = 0u64;
    loop {
        let Ok((m, from)) = s.recv_from(&mut buf) else { continue };
        n += 1;
        let d = sha256(&buf[..m]);
        println!(
            "{}",
            json!({"ev":"rx","n":n,"from":from.to_string(),"len":m,"sha":hex(&d),"shaped":shaped(&buf[..m]),"head":hex(&buf[..m.min(8)])})
        );
        let mut ack = Vec::with_capacity(36);
        ack.extend_from_slice(&(m as u32).to_be_bytes());
        ack.extend_from_slice(&d);
        let _ = s.send_to(&ack, from);
    }
}
