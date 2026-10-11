//! Two-pump bidirectional forwarder between two legs. Mirrors crates/overdrive-dataplane/src/mtls/splice.rs:
//! one thread per direction, `poll` then (blocking) `splice(src -> pipe -> dst)` with SPLICE_F_MOVE only,
//! or a plain read/write copy. EOF is forwarded as shutdown(dst, SHUT_WR) (bare FIN on kTLS, no
//! close_notify, like v1 today); errors / vsock full-close abort both legs (SO_LINGER 0 -> RST).
use crate::app::log;
use crate::sys::*;
use crate::tlsk;
use crate::tp;
use serde_json::{json, Value};
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind { Tcp, Ktls, Vsock }
#[derive(Clone, Copy)]
pub struct Leg { pub fd: RawFd, pub kind: Kind, pub names: Option<(u32, u32, u32)> }
impl Leg {
    /// `names` (vsock peer cid/port, local port) are read once, while the flow is still connected:
    /// getpeername fails once the peer has closed.
    pub fn new(fd: RawFd, kind: Kind) -> Leg {
        Leg { fd, kind, names: if kind == Kind::Vsock { vsock_names(fd) } else { None } }
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fwd { Splice, Copy }

struct Shared { abort: AtomicBool, legs: [Leg; 2] }
impl Shared {
    fn abort_all(&self) {
        if !self.abort.swap(true, Ordering::SeqCst) {
            for l in &self.legs {
                if l.kind != Kind::Vsock { linger0(l.fd); }
            }
        }
    }
}

fn sp(fd_in: RawFd, fd_out: RawFd, len: usize) -> isize {
    unsafe { libc::splice(fd_in, std::ptr::null_mut(), fd_out, std::ptr::null_mut(), len, libc::SPLICE_F_MOVE) }
}

fn pump(src: Leg, dst: Leg, fwd: Fwd, sh: &Shared, name: &str, id: u64) -> Value {
    let (mut pr, mut pw) = (-1, -1);
    if fwd == Fwd::Splice {
        let mut p = [0i32; 2];
        unsafe { libc::pipe2(p.as_mut_ptr(), libc::O_CLOEXEC) };
        unsafe { libc::fcntl(p[0], 1031, 1 << 20) }; // F_SETPIPE_SZ
        pr = p[0];
        pw = p[1];
    }
    let mut buf = vec![0u8; if fwd == Fwd::Copy { 256 << 10 } else { 0 }];
    let mut bytes = 0u64;
    let mut einval = 0u64;
    let end: String;
    let mut errno_v = 0;
    'outer: loop {
        if sh.abort.load(Ordering::SeqCst) {
            end = "stopped_by_abort".into();
            break;
        }
        if poll1(src.fd, libc::POLLIN | libc::POLLRDHUP, 100) == 0 {
            continue;
        }
        let n: isize = match fwd {
            Fwd::Splice => sp(src.fd, pw, 1 << 20),
            Fwd::Copy => rd(src.fd, &mut buf),
        };
        if n > 0 {
            match fwd {
                Fwd::Splice => {
                    let mut rem = n;
                    while rem > 0 {
                        let m = sp(pr, dst.fd, rem as usize);
                        if m <= 0 {
                            let e = if m < 0 { errno() } else { 0 };
                            if m < 0 && e == libc::EINTR { continue; }
                            errno_v = e;
                            sh.abort_all();
                            end = format!("write_err:{e}");
                            break 'outer;
                        }
                        rem -= m;
                    }
                }
                Fwd::Copy => {
                    if let Err(e) = write_all(dst.fd, &buf[..n as usize]) {
                        errno_v = e;
                        sh.abort_all();
                        end = format!("write_err:{e}");
                        break 'outer;
                    }
                }
            }
            bytes += n as u64;
        } else if n == 0 {
            let clean = match src.kind {
                Kind::Vsock => {
                    let z = zero_send(src.fd);
                    let mode = EOF_CLASS.load(Ordering::SeqCst);
                    log(json!({"ev": "vsock_eof_classify", "id": id, "dir": name, "zero_send_errno": z, "mode": mode}));
                    match mode {
                        1 => z == 0,
                        2 => match src.names {
                            Some((pc, pp, lp)) => {
                                let f = tp::first_from_peer(pc, pp, lp);
                                log(json!({"ev": "tp_classify", "id": id, "dir": name, "first": format!("{f:?}")}));
                                tp::forget(pc, pp, lp);
                                matches!(f, Some((tp::First::Half, _)) | None)
                            }
                            None => true,
                        },
                        _ => true,
                    }
                }
                _ => true,
            };
            if clean {
                shutdown_wr(dst.fd);
                end = "eof_clean".into();
            } else {
                sh.abort_all();
                end = "eof_abort".into();
            }
            break;
        } else {
            let e = errno();
            if e == libc::EINTR || e == libc::EAGAIN { continue; }
            if src.kind == Kind::Ktls && (e == libc::EINVAL || e == libc::EIO) {
                einval += 1;
                let mut t = [0u8; 64];
                match tlsk::recv_record(src.fd, &mut t) {
                    Ok((21, 2)) if t[1] == 0 => {
                        shutdown_wr(dst.fd);
                        end = "close_notify".into();
                    }
                    Ok((typ, n)) => {
                        sh.abort_all();
                        end = format!("ctrl_record:{typ}:{n}");
                    }
                    Err(e2) => {
                        sh.abort_all();
                        end = format!("ctrl_read_err:{e2}");
                    }
                }
                break;
            }
            errno_v = e;
            sh.abort_all();
            end = format!("read_err:{e}");
            break;
        }
    }
    if pr >= 0 { unsafe { libc::close(pr); libc::close(pw); } }
    log(json!({"ev": "pump_end", "id": id, "dir": name, "bytes": bytes, "end": end, "errno": errno_v, "splice_einval": einval, "t_us": now_us()}));
    json!({"bytes": bytes, "end": end})
}

pub fn duplex(a: Leg, b: Leg, fwd: Fwd, id: u64) -> Value {
    let sh = Arc::new(Shared { abort: AtomicBool::new(false), legs: [a, b] });
    let (s1, s2) = (sh.clone(), sh.clone());
    let t1 = std::thread::spawn(move || pump(a, b, fwd, &s1, "c2s", id));
    let t2 = std::thread::spawn(move || pump(b, a, fwd, &s2, "s2c", id));
    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    json!({"c2s": r1, "s2c": r2, "aborted": sh.abort.load(Ordering::SeqCst)})
}
