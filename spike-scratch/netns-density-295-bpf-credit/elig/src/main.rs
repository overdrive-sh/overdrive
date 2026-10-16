//! bpf-credit eligibility probe (spike; throwaway; Rust + libc only, no C).
//!
//! Question: can ANY BPF program type attach to / override the vsock credit-return path of the stock
//! kernel? The answer is decided by the verifier and the attach machinery, not by the program body, so
//! each row loads a minimal program through the raw bpf(2) syscall with the exact (prog_type,
//! expected_attach_type, attach_btf_id, attach_btf_obj_fd) the real program would use, and records what
//! the kernel says. Rows: fentry / fexit / fmod_ret per function, a BTF-write attempt, kprobe +
//! bpf_override_return. Positive controls (an error-injectable function, a read-only fentry) prove the
//! probe itself can succeed. Also lists the struct_ops types of the running kernel.
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;

const BPF_PROG_LOAD: u32 = 5;
const BPF_OBJ_GET_INFO_BY_FD: u32 = 15;
const BPF_RAW_TRACEPOINT_OPEN: u32 = 17;
const BPF_BTF_GET_FD_BY_ID: u32 = 19;
const BPF_BTF_GET_NEXT_ID: u32 = 23;

const PT_KPROBE: u32 = 2;
const PT_TRACING: u32 = 26;
const AT_FENTRY: u32 = 24;
const AT_FEXIT: u32 = 25;
const AT_MODRET: u32 = 26;

fn sys_bpf(cmd: u32, attr: &mut [u8]) -> i64 {
    unsafe { libc::syscall(libc::SYS_bpf, cmd as libc::c_long, attr.as_mut_ptr(), attr.len()) as i64 }
}
fn errno() -> i32 { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) }
fn put32(b: &mut [u8], off: usize, v: u32) { b[off..off + 4].copy_from_slice(&v.to_le_bytes()); }
fn put64(b: &mut [u8], off: usize, v: u64) { b[off..off + 8].copy_from_slice(&v.to_le_bytes()); }
fn rd32(b: &[u8], off: usize) -> u32 { u32::from_le_bytes(b[off..off + 4].try_into().unwrap()) }
fn rd16(b: &[u8], off: usize) -> u16 { u16::from_le_bytes(b[off..off + 2].try_into().unwrap()) }

// ------------------------------------------------------------------ BTF
struct Btf {
    first_id: u32,
    ntypes: u32,
    funcs: BTreeMap<String, (u32, u32)>, // name -> (btf id, linkage)
    structs: Vec<String>,
}

fn cstr(b: &[u8], off: usize) -> String {
    let e = b[off..].iter().position(|&c| c == 0).map(|p| off + p).unwrap_or(b.len());
    String::from_utf8_lossy(&b[off..e]).into_owned()
}

fn parse_btf(d: &[u8], first_id: u32, base_strs: &[u8]) -> (Btf, Vec<u8>) {
    assert_eq!(rd16(d, 0), 0xEB9F, "btf magic");
    let hdr = rd32(d, 4) as usize;
    let (to, tl, so) = (rd32(d, 8) as usize, rd32(d, 12) as usize, rd32(d, 16) as usize);
    let types = &d[hdr + to..hdr + to + tl];
    let own = &d[hdr + so..hdr + so + rd32(d, 20) as usize];
    let name_of = |off: usize| -> String { if off < base_strs.len() { cstr(base_strs, off) } else { cstr(own, off - base_strs.len()) } };
    let mut b = Btf { first_id, ntypes: 0, funcs: BTreeMap::new(), structs: vec![] };
    let (mut p, mut id) = (0usize, first_id);
    while p + 12 <= types.len() {
        let name_off = rd32(types, p) as usize;
        let info = rd32(types, p + 4);
        let (kind, vlen) = ((info >> 24) & 0x1f, (info & 0xffff) as usize);
        let extra = match kind {
            1 => 4, 3 => 12, 4 | 5 => vlen * 12, 6 => vlen * 8, 13 => vlen * 8, 14 => 4, 15 => vlen * 12,
            17 => 4, 19 => vlen * 12, _ => 0,
        };
        if kind == 12 { b.funcs.insert(name_of(name_off), (id, vlen as u32)); }
        if kind == 4 { let n = name_of(name_off); if n.starts_with("bpf_struct_ops_") { b.structs.push(n); } }
        p += 12 + extra; id += 1; b.ntypes += 1;
    }
    (b, own.to_vec())
}

/// (module name -> (btf id, fd))
fn kernel_btf_objects() -> BTreeMap<String, (u32, i32)> {
    let mut out = BTreeMap::new();
    let mut start = 0u32;
    loop {
        let mut a = [0u8; 16]; put32(&mut a, 0, start);
        if sys_bpf(BPF_BTF_GET_NEXT_ID, &mut a) < 0 { break; }
        let id = rd32(&a, 4); start = id;
        let mut g = [0u8; 16]; put32(&mut g, 0, id);
        let fd = sys_bpf(BPF_BTF_GET_FD_BY_ID, &mut g);
        if fd < 0 { continue; }
        let mut name = [0u8; 64];
        let mut info = [0u8; 32];
        put64(&mut info, 16, name.as_mut_ptr() as u64); put32(&mut info, 24, 64);
        let mut q = [0u8; 16]; put32(&mut q, 0, fd as u32); put32(&mut q, 4, 32); put64(&mut q, 8, info.as_mut_ptr() as u64);
        if sys_bpf(BPF_OBJ_GET_INFO_BY_FD, &mut q) == 0 && rd32(&info, 28) == 1 {
            let n = cstr(&name, 0);
            out.insert(n, (id, fd as i32));
        } else { unsafe { libc::close(fd as i32); } }
    }
    out
}

// ------------------------------------------------------------------ insns
fn insn(code: u8, dst: u8, src: u8, off: i16, imm: i32) -> [u8; 8] {
    let mut b = [0u8; 8]; b[0] = code; b[1] = (src << 4) | dst; b[2..4].copy_from_slice(&off.to_le_bytes()); b[4..8].copy_from_slice(&imm.to_le_bytes()); b
}
fn prog(insns: &[[u8; 8]]) -> Vec<u8> { insns.iter().flatten().copied().collect() }
fn p_ret0() -> Vec<u8> { prog(&[insn(0xb7, 0, 0, 0, 0), insn(0x95, 0, 0, 0, 0)]) }
/// r2 = ctx[0] (first traced argument, a PTR_TO_BTF_ID); r3 = *(u32*)(r2+0) (probe-read semantics); r0 = 0
fn p_btf_read() -> Vec<u8> { prog(&[insn(0x79, 2, 1, 0, 0), insn(0x61, 3, 2, 0, 0), insn(0xb7, 0, 0, 0, 0), insn(0x95, 0, 0, 0, 0)]) }
/// r2 = ctx[0]; *(u32*)(r2+0) = 0  (a WRITE through the traced pointer); r0 = 0
fn p_btf_write() -> Vec<u8> { prog(&[insn(0x79, 2, 1, 0, 0), insn(0x62, 2, 0, 0, 0), insn(0xb7, 0, 0, 0, 0), insn(0x95, 0, 0, 0, 0)]) }
/// r2 = 0; call bpf_override_return(ctx, 0); r0 = 0
fn p_override() -> Vec<u8> { prog(&[insn(0xb7, 2, 0, 0, 0), insn(0x85, 0, 0, 0, 58), insn(0xb7, 0, 0, 0, 0), insn(0x95, 0, 0, 0, 0)]) }

struct Loaded { fd: i32, log: String }
fn load(ptype: u32, eat: u32, btf_id: u32, btf_obj_fd: i32, code: &[u8]) -> Result<Loaded, (i32, String)> {
    let mut log = vec![0u8; 1 << 16];
    let lic = CString::new("GPL").unwrap();
    let mut a = [0u8; 256];
    put32(&mut a, 0, ptype); put32(&mut a, 4, (code.len() / 8) as u32); put64(&mut a, 8, code.as_ptr() as u64);
    put64(&mut a, 16, lic.as_ptr() as u64); put32(&mut a, 24, 1); put32(&mut a, 28, log.len() as u32); put64(&mut a, 32, log.as_mut_ptr() as u64);
    put32(&mut a, 68, eat); put32(&mut a, 108, btf_id); put32(&mut a, 112, if btf_obj_fd > 0 { btf_obj_fd as u32 } else { 0 });
    let r = sys_bpf(BPF_PROG_LOAD, &mut a);
    let e = errno();
    let l = cstr(&log, 0);
    let tail: String = { let t = l.trim_end(); let lines: Vec<&str> = t.lines().collect(); lines[lines.len().saturating_sub(3)..].join(" | ") };
    if r >= 0 { Ok(Loaded { fd: r as i32, log: tail }) } else { Err((e, tail)) }
}

fn attach_raw(fd: i32) -> Result<i32, i32> {
    let mut a = [0u8; 16]; put32(&mut a, 8, fd as u32);
    let r = sys_bpf(BPF_RAW_TRACEPOINT_OPEN, &mut a);
    if r >= 0 { Ok(r as i32) } else { Err(errno()) }
}

fn kprobe_attach(fd: i32, func: &str) -> Result<(), i32> {
    let ty: u32 = fs::read_to_string("/sys/bus/event_source/devices/kprobe/type").map_err(|_| -1)?.trim().parse().map_err(|_| -2)?;
    let name = CString::new(func).unwrap();
    let mut at = [0u8; 128];
    put32(&mut at, 0, ty); put32(&mut at, 4, 128); put64(&mut at, 56, name.as_ptr() as u64); put64(&mut at, 64, 0);
    let pfd = unsafe { libc::syscall(libc::SYS_perf_event_open, at.as_ptr(), -1i32, 0i32, -1i32, 8u64) } as i32;
    if pfd < 0 { return Err(errno()); }
    let r = unsafe { libc::ioctl(pfd, 0x40042408 as _, fd) };
    let e = errno();
    unsafe { libc::close(pfd); }
    if r < 0 { Err(e) } else { Ok(()) }
}

fn esc(s: &str) -> String { s.replace('\\', "\\\\").replace('"', "'") }
fn errname(e: i32) -> String { format!("{} ({})", e, std::io::Error::from_raw_os_error(e)) }

fn main() {
    let uname = { let mut u: libc::utsname = unsafe { std::mem::zeroed() }; unsafe { libc::uname(&mut u); } cstr(unsafe { &*(u.release.as_ptr() as *const [u8; 65]) }, 0) };
    println!("{{\"row\":\"env\",\"uname_r\":\"{}\"}}", uname);
    let vm = fs::read("/sys/kernel/btf/vmlinux").expect("vmlinux btf");
    let (base, base_strs) = parse_btf(&vm, 1, &[]);
    let base_cnt = base.first_id + base.ntypes; // = btf__type_cnt(base)
    println!("{{\"row\":\"btf\",\"vmlinux_types\":{},\"vmlinux_funcs\":{}}}", base.ntypes, base.funcs.len());
    let objs = kernel_btf_objects();
    let mut mods: BTreeMap<String, Btf> = BTreeMap::new();
    for m in ["vsock", "vmw_vsock_virtio_transport_common", "vmw_vsock_virtio_transport", "vhost_vsock", "vsock_loopback", "vhost"] {
        if let Ok(d) = fs::read(format!("/sys/kernel/btf/{m}")) {
            let (b, _) = parse_btf(&d, base_cnt, &base_strs);
            println!("{{\"row\":\"btf_module\",\"module\":\"{m}\",\"types\":{},\"funcs\":{},\"loaded_btf_obj\":{}}}", b.ntypes, b.funcs.len(), objs.contains_key(m));
            mods.insert(m.to_string(), b);
        } else { println!("{{\"row\":\"btf_module\",\"module\":\"{m}\",\"present\":false}}"); }
    }
    let ksyms = fs::read_to_string("/proc/kallsyms").unwrap_or_default();
    let ksym = |f: &str| -> Option<String> {
        ksyms.lines().find_map(|l| { let mut it = l.split_whitespace(); let addr = it.next()?; let _t = it.next()?; let n = it.next()?; let m = it.next().map(|s| s.to_string()); if n == f { Some(format!("{addr} {}", m.unwrap_or_else(|| "[vmlinux]".into()))) } else { None } })
    };
    // struct_ops types the running kernel can register
    let mut so: Vec<String> = base.structs.clone();
    for (m, b) in &mods { for s in &b.structs { so.push(format!("{s}@{m}")); } }
    println!("{{\"row\":\"struct_ops_btf_types\",\"types\":{:?}}}", so);

    // (function, where to look). "vmlinux" or a module.
    let targets: &[(&str, &str)] = &[
        ("virtio_transport_send_credit_update", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_dec_rx_pkt", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_read_skb", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_inc_tx_pkt", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_recv_pkt", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_notify_set_rcvlowat", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_get_credit", "vmw_vsock_virtio_transport_common"),
        ("virtio_transport_recv_enqueue", "vmw_vsock_virtio_transport_common"),
        ("vsock_data_ready", "vsock"),
        ("vsock_set_rcvlowat", "vsock"),
        ("sk_psock_verdict_recv", "vmlinux"),
        ("sk_psock_verdict_data_ready", "vmlinux"),
        ("sk_psock_skb_redirect", "vmlinux"),
        ("sk_psock_verdict_apply", "vmlinux"),
        ("sk_psock_backlog", "vmlinux"),
        ("sk_psock_handle_skb", "vmlinux"),
        ("skb_send_sock", "vmlinux"),
        ("tcp_read_skb", "vmlinux"),
        // controls: a security_* hook (fmod_ret allowed by name), error-injectable entries, a page_pool entry
        ("security_socket_recvmsg", "vmlinux"),
        ("security_socket_sendmsg", "vmlinux"),
        ("page_pool_alloc_netmems", "vmlinux"),
        ("__x64_sys_iopl", "vmlinux"),
        ("__x64_sys_sendto", "vmlinux"),
    ];
    for (f, w) in targets {
        let (btf, objfd): (Option<(u32, u32)>, i32) = if *w == "vmlinux" {
            (base.funcs.get(*f).copied(), 0)
        } else {
            (mods.get(*w).and_then(|b| b.funcs.get(*f).copied()), objs.get(*w).map(|x| x.1).unwrap_or(0))
        };
        let ks = ksym(f);
        println!("{{\"row\":\"symbol\",\"func\":\"{f}\",\"where\":\"{w}\",\"btf_id\":{},\"linkage\":\"{}\",\"kallsyms\":\"{}\"}}",
                 btf.map(|x| x.0 as i64).unwrap_or(-1),
                 btf.map(|x| match x.1 { 0 => "static", 1 => "global", _ => "extern" }).unwrap_or("-"),
                 ks.clone().unwrap_or_else(|| "absent".into()));
        if let Some((id, _)) = btf {
        for (mode, eat, code) in [("fentry", AT_FENTRY, p_ret0()), ("fexit", AT_FEXIT, p_ret0()), ("fmod_ret", AT_MODRET, p_ret0())] {
            match load(PT_TRACING, eat, id, objfd, &code) {
                Ok(l) => {
                    let att = attach_raw(l.fd);
                    let (ar, ae) = match att { Ok(lfd) => { unsafe { libc::close(lfd); } ("attached".to_string(), String::new()) } Err(e) => ("attach_failed".to_string(), errname(e)) };
                    unsafe { libc::close(l.fd); }
                    println!("{{\"row\":\"tracing\",\"func\":\"{f}\",\"mode\":\"{mode}\",\"load\":\"ok\",\"attach\":\"{ar}\",\"attach_err\":\"{}\"}}", esc(&ae));
                }
                Err((e, log)) => println!("{{\"row\":\"tracing\",\"func\":\"{f}\",\"mode\":\"{mode}\",\"load\":\"rejected\",\"errno\":\"{}\",\"verifier\":\"{}\"}}", esc(&errname(e)), esc(&log)),
            }
        }
        // fentry that READS the first traced pointer (control) and one that WRITES through it
        for (mode, code) in [("fentry-btf-read", p_btf_read()), ("fentry-btf-write", p_btf_write())] {
            match load(PT_TRACING, AT_FENTRY, id, objfd, &code) {
                Ok(l) => { unsafe { libc::close(l.fd); } println!("{{\"row\":\"tracing\",\"func\":\"{f}\",\"mode\":\"{mode}\",\"load\":\"ok\"}}"); }
                Err((e, log)) => println!("{{\"row\":\"tracing\",\"func\":\"{f}\",\"mode\":\"{mode}\",\"load\":\"rejected\",\"errno\":\"{}\",\"verifier\":\"{}\"}}", esc(&errname(e)), esc(&log)),
            }
        }
        }
        // kprobe + bpf_override_return (needs the function on the opt-in list AND at function entry)
        if ks.is_some() {
            match load(PT_KPROBE, 0, 0, 0, &p_override()) {
                Ok(l) => {
                    let r = kprobe_attach(l.fd, f);
                    unsafe { libc::close(l.fd); }
                    match r {
                        Ok(()) => println!("{{\"row\":\"kprobe_override\",\"func\":\"{f}\",\"load\":\"ok\",\"attach\":\"ok\"}}"),
                        Err(e) => println!("{{\"row\":\"kprobe_override\",\"func\":\"{f}\",\"load\":\"ok\",\"attach\":\"rejected\",\"errno\":\"{}\"}}", esc(&errname(e))),
                    }
                }
                Err((e, log)) => println!("{{\"row\":\"kprobe_override\",\"func\":\"{f}\",\"load\":\"rejected\",\"errno\":\"{}\",\"verifier\":\"{}\"}}", esc(&errname(e)), esc(&log)),
            }
        } else {
            println!("{{\"row\":\"kprobe_override\",\"func\":\"{f}\",\"load\":\"skipped\",\"why\":\"symbol not in kallsyms\"}}");
        }
    }
}
