//! V-6 probe: can userspace read back the exact programmed backend set of a frontend from the
//! kernel (outer SERVICE_MAP slot -> inner map id -> BPF_MAP_GET_FD_BY_ID -> inner array ->
//! BACKEND_MAP)? THROWAWAY. Real `EbpfDataplane` programs the maps through `update_service`;
//! this binary only READS, with raw bpf(2) syscalls it owns (nothing under crates/ is touched).
#![allow(clippy::all, clippy::pedantic, dead_code)]

use std::collections::BTreeSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::id::{ServiceVip, SpiffeId};
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_dataplane::EbpfDataplane;

const INNER_CAP: u32 = 16_381;
const BPF_MAP_LOOKUP_ELEM: i64 = 1;
const BPF_OBJ_GET: i64 = 7;
const BPF_MAP_GET_NEXT_ID: i64 = 12;
const BPF_MAP_GET_FD_BY_ID: i64 = 14;
const BPF_OBJ_GET_INFO_BY_FD: i64 = 15;
const BPF_MAP_LOOKUP_BATCH: i64 = 24;
const BPF_F_RDONLY: u32 = 8;

// ---------------------------------------------------------------- raw bpf(2)
fn sys(cmd: i64, attr: &mut [u8; 128]) -> io::Result<i64> {
    let r = unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), 128usize) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}
fn put32(a: &mut [u8; 128], off: usize, v: u32) { a[off..off + 4].copy_from_slice(&v.to_ne_bytes()); }
fn put64(a: &mut [u8; 128], off: usize, v: u64) { a[off..off + 8].copy_from_slice(&v.to_ne_bytes()); }
fn get32(a: &[u8; 128], off: usize) -> u32 { u32::from_ne_bytes(a[off..off + 4].try_into().unwrap()) }
fn enoent(e: &io::Error) -> bool { e.raw_os_error() == Some(libc::ENOENT) }

fn obj_get(path: &Path) -> io::Result<OwnedFd> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let mut a = [0u8; 128];
    put64(&mut a, 0, c.as_ptr() as u64);
    let fd = sys(BPF_OBJ_GET, &mut a)?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
/// Ok(true) hit, Ok(false) ENOENT.
fn lookup_into(fd: i32, key: &[u8], val: &mut [u8]) -> io::Result<bool> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, fd as u32);
    put64(&mut a, 8, key.as_ptr() as u64);
    put64(&mut a, 16, val.as_mut_ptr() as u64);
    match sys(BPF_MAP_LOOKUP_ELEM, &mut a) {
        Ok(_) => Ok(true),
        Err(e) if enoent(&e) => Ok(false),
        Err(e) => Err(e),
    }
}
fn get_fd_by_id(id: u32) -> io::Result<OwnedFd> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, id);
    put32(&mut a, 8, BPF_F_RDONLY);
    let fd = sys(BPF_MAP_GET_FD_BY_ID, &mut a)?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
struct MapInfo { ty: u32, id: u32, key_size: u32, value_size: u32, max_entries: u32, name: String }
fn info_by_fd(fd: i32) -> io::Result<MapInfo> {
    let mut info = [0u8; 128];
    let mut a = [0u8; 128];
    put32(&mut a, 0, fd as u32);
    put32(&mut a, 4, 128);
    put64(&mut a, 8, info.as_mut_ptr() as u64);
    sys(BPF_OBJ_GET_INFO_BY_FD, &mut a)?;
    let nm = &info[24..40];
    let end = nm.iter().position(|b| *b == 0).unwrap_or(16);
    Ok(MapInfo {
        ty: u32::from_ne_bytes(info[0..4].try_into().unwrap()),
        id: u32::from_ne_bytes(info[4..8].try_into().unwrap()),
        key_size: u32::from_ne_bytes(info[8..12].try_into().unwrap()),
        value_size: u32::from_ne_bytes(info[12..16].try_into().unwrap()),
        max_entries: u32::from_ne_bytes(info[16..20].try_into().unwrap()),
        name: String::from_utf8_lossy(&nm[..end]).into_owned(),
    })
}
fn maps_named(name: &str) -> io::Result<Vec<(u32, OwnedFd)>> {
    let mut out = vec![];
    let mut id = 0u32;
    loop {
        let mut a = [0u8; 128];
        put32(&mut a, 0, id);
        match sys(BPF_MAP_GET_NEXT_ID, &mut a) {
            Ok(_) => id = get32(&a, 4),
            Err(e) if enoent(&e) => break,
            Err(e) => return Err(e),
        }
        let fd = match get_fd_by_id(id) { Ok(f) => f, Err(e) if enoent(&e) => continue, Err(e) => return Err(e) };
        if info_by_fd(fd.as_raw_fd())?.name == name { out.push((id, fd)); }
    }
    Ok(out)
}
/// One BPF_MAP_LOOKUP_BATCH call. Returns (count, finished(ENOENT)).
fn batch_call(fd: i32, in_batch: Option<&[u8; 4]>, out_batch: &mut [u8; 4], keys: &mut [u32], vals: &mut [u32]) -> io::Result<(u32, bool)> {
    let mut a = [0u8; 128];
    put64(&mut a, 0, in_batch.map_or(0, |b| b.as_ptr() as u64));
    put64(&mut a, 8, out_batch.as_mut_ptr() as u64);
    put64(&mut a, 16, keys.as_mut_ptr() as u64);
    put64(&mut a, 24, vals.as_mut_ptr() as u64);
    put32(&mut a, 32, keys.len() as u32);
    put32(&mut a, 36, fd as u32);
    let r = sys(BPF_MAP_LOOKUP_BATCH, &mut a);
    let n = get32(&a, 32);
    match r { Ok(_) => Ok((n, false)), Err(e) if enoent(&e) => Ok((n, true)), Err(e) => Err(e) }
}

// ---------------------------------------------------------------- the read under test
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Pb { addr: SocketAddrV4, weight: u16, healthy: bool }

#[derive(Debug)]
enum ReadErr { InnerGone(u32), BackendMissing(u32), Sys(io::Error) }
impl From<io::Error> for ReadErr { fn from(e: io::Error) -> Self { ReadErr::Sys(e) } }

#[derive(Clone, Copy, Debug)]
enum Strat { Single, Batch(u32) }

#[derive(Default, Clone, Copy, Debug)]
struct Steps { outer: Duration, getfd: Duration, inner: Duration, backend: Duration, distinct: usize }

struct Reader { outer: OwnedFd, backend: OwnedFd, backend_id: u32 }
impl Reader {
    fn new(pin_dir: &Path) -> io::Result<Self> {
        let outer = obj_get(&pin_dir.join("SERVICE_MAP"))?;
        let mut bm = maps_named("BACKEND_MAP")?;
        if bm.len() != 1 {
            return Err(io::Error::other(format!("expected exactly one BACKEND_MAP, found {:?}", bm.iter().map(|x| x.0).collect::<Vec<_>>())));
        }
        let (backend_id, backend) = bm.remove(0);
        Ok(Self { outer, backend, backend_id })
    }
    fn key(f: &ServiceFrontend) -> [u8; 8] {
        let mut k = [0u8; 8];
        k[0..4].copy_from_slice(&u32::from(f.vip_v4()).to_ne_bytes());
        k[4..6].copy_from_slice(&f.port().get().to_ne_bytes());
        k[6] = f.proto().as_u8();
        k
    }
    /// Step 1: outer lookup -> inner map ID (4 bytes), `None` = no slot.
    fn outer_id(&self, f: &ServiceFrontend) -> io::Result<Option<u32>> {
        let mut v = [0u8; 4];
        Ok(lookup_into(self.outer.as_raw_fd(), &Self::key(f), &mut v)?.then(|| u32::from_ne_bytes(v)))
    }
    fn read(&self, f: &ServiceFrontend, strat: Strat) -> Result<(BTreeSet<Pb>, Steps), ReadErr> {
        let mut st = Steps::default();
        let t0 = Instant::now();
        let Some(id) = self.outer_id(f)? else { return Ok((BTreeSet::new(), st)) };
        st.outer = t0.elapsed();
        let t1 = Instant::now();
        let inner = match get_fd_by_id(id) { Ok(fd) => fd, Err(e) if enoent(&e) => return Err(ReadErr::InnerGone(id)), Err(e) => return Err(e.into()) };
        st.getfd = t1.elapsed();
        let t2 = Instant::now();
        let mut ids: Vec<u32> = Vec::with_capacity(INNER_CAP as usize);
        match strat {
            Strat::Single => {
                let mut v = [0u8; 4];
                for slot in 0..INNER_CAP {
                    if lookup_into(inner.as_raw_fd(), &slot.to_ne_bytes(), &mut v)? { ids.push(u32::from_ne_bytes(v)); }
                }
            }
            Strat::Batch(chunk) => {
                let mut keys = vec![0u32; chunk as usize];
                let mut vals = vec![0u32; chunk as usize];
                let mut prev = [0u8; 4];
                let mut out = [0u8; 4];
                let mut first = true;
                loop {
                    let (n, done) = batch_call(inner.as_raw_fd(), if first { None } else { Some(&prev) }, &mut out, &mut keys, &mut vals)?;
                    ids.extend_from_slice(&vals[..n as usize]);
                    if done { break; }
                    prev = out; first = false;
                }
            }
        }
        st.inner = t2.elapsed();
        let t3 = Instant::now();
        ids.sort_unstable();
        ids.dedup();
        st.distinct = ids.len();
        let mut set = BTreeSet::new();
        let mut v = [0u8; 12];
        for bid in ids {
            if bid == 0 { continue; }
            if !lookup_into(self.backend.as_raw_fd(), &bid.to_ne_bytes(), &mut v)? { return Err(ReadErr::BackendMissing(bid)); }
            let ip = u32::from_ne_bytes(v[0..4].try_into().unwrap());
            let port = u16::from_ne_bytes(v[4..6].try_into().unwrap());
            let weight = u16::from_ne_bytes(v[6..8].try_into().unwrap());
            set.insert(Pb { addr: SocketAddrV4::new(Ipv4Addr::from(ip), port), weight, healthy: v[8] != 0 });
        }
        st.backend = t3.elapsed();
        Ok((set, st))
    }
}

// ---------------------------------------------------------------- fixtures
fn frontend(vip: Ipv4Addr, port: u16, proto: Proto) -> ServiceFrontend {
    ServiceFrontend::new(ServiceVip::new(IpAddr::V4(vip)).unwrap(), port.try_into().unwrap(), proto).unwrap()
}
fn backend(ip: Ipv4Addr, port: u16, weight: u16, healthy: bool) -> Backend {
    let alloc = SpiffeId::new(&format!("spiffe://overdrive.local/workload/spk/alloc/b-{}-{port}", ip.to_string().replace('.', "-"))).unwrap();
    Backend { alloc, addr: SocketAddr::new(IpAddr::V4(ip), port), weight, healthy }
}
fn project(b: &[Backend]) -> BTreeSet<Pb> {
    b.iter().filter_map(|x| match x.addr { SocketAddr::V4(a) => Some(Pb { addr: a, weight: x.weight, healthy: x.healthy }), _ => None }).collect()
}
struct Lcg(u64);
impl Lcg { fn next(&mut self) -> u64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); self.0 >> 33 } }
fn ip_at(base: u32, i: u32) -> Ipv4Addr { Ipv4Addr::from(base + i) }
fn random_set(rng: &mut Lcg, base: u32, n: u32, port: u16) -> Vec<Backend> {
    (0..n).map(|i| {
        let w = if std::env::var("SPK_WEIGHTS").as_deref() == Ok("prod") { let _ = rng.next(); 1 } else { match rng.next() % 6 { 0 => 0, 1 => 1, 2 => 65535, _ => (rng.next() % 1000) as u16 } };
        backend(ip_at(base, i), port, w, rng.next() % 3 != 0)
    }).collect()
}

fn make_dp(pin_dir: &Path) -> Arc<EbpfDataplane> {
    Arc::new(EbpfDataplane::new_with_pin_dir("spkv0", "spkv1", pin_dir, Path::new("/sys/fs/cgroup/spike-psmr")).expect("EbpfDataplane::new_with_pin_dir"))
}
fn pin_dir_arg() -> PathBuf { PathBuf::from(std::env::var("SPK_PIN_DIR").expect("SPK_PIN_DIR")) }

fn stats(mut v: Vec<Duration>) -> String {
    v.sort();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize];
    format!("n={} min={:.0}us p50={:.0}us p99={:.0}us max={:.0}us", v.len(), us(v[0]), us(p(0.5)), us(p(0.99)), us(*v.last().unwrap()))
}
fn p50(v: Vec<Duration>) -> String { let mut v = v; v.sort(); format!("{:.0}us", v[v.len() / 2].as_secs_f64() * 1e6) }
fn check(name: &str, ok: bool) { println!("{} {name}", if ok { "PASS" } else { "FAIL" }); if !ok { std::process::exit(1) } }

// ---------------------------------------------------------------- scenarios
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "correctness" => correctness().await,
        "race" => { let secs = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8); std::thread::spawn(move || race(secs)).join().unwrap() }
        "restart" => restart().await,
        "cost" => cost().await,
        "compare" => compare(),
        _ => eprintln!("usage: correctness | race [secs] | restart | cost"),
    }
}

async fn correctness() {
    let pin = pin_dir_arg();
    let dp = make_dp(&pin);
    let rd = Reader::new(&pin).unwrap();
    let mut rng = Lcg(42);
    let strats = [Strat::Single, Strat::Batch(INNER_CAP), Strat::Batch(1024)];

    println!("== K-1: outer lookup returns an inner map ID; K-2: GET_FD_BY_ID gives a readable fd");
    let f = frontend(Ipv4Addr::new(10, 96, 0, 1), 80, Proto::Tcp);
    check("never-programmed frontend reads empty (no slot)", rd.read(&f, Strat::Batch(INNER_CAP)).unwrap().0.is_empty() && rd.outer_id(&f).unwrap().is_none());
    let b = random_set(&mut rng, 0x0A80_0000, 16, 8080);
    dp.update_service(f.clone(), b.clone()).await.unwrap();
    let id1 = rd.outer_id(&f).unwrap().expect("slot");
    let inner = get_fd_by_id(id1).unwrap();
    let ii = info_by_fd(inner.as_raw_fd()).unwrap();
    println!("outer value = inner map id {id1}; GET_FD_BY_ID(RDONLY) ok; inner info: type={} (2=ARRAY) id={} key_size={} value_size={} max_entries={} name={:?}", ii.ty, ii.id, ii.key_size, ii.value_size, ii.max_entries, ii.name);
    check("inner is BPF_MAP_TYPE_ARRAY u32->u32 with 16381 entries, id == outer value", ii.ty == 2 && ii.key_size == 4 && ii.value_size == 4 && ii.max_entries == INNER_CAP && ii.id == id1);
    let oi = info_by_fd(rd.outer.as_raw_fd()).unwrap();
    println!("outer info: type={} (13=HASH_OF_MAPS) key_size={} value_size={} max_entries={} name={:?}", oi.ty, oi.key_size, oi.value_size, oi.max_entries, oi.name);

    println!("== exactness for 1 / 16 / 256 backends, every strategy");
    for n in [1u32, 16, 256] {
        let b = random_set(&mut rng, 0x0A80_0000 + n * 1000, n, 8080);
        dp.update_service(f.clone(), b.clone()).await.unwrap();
        let want = project(&b);
        for s in strats {
            let (got, st) = rd.read(&f, s).unwrap();
            println!("n={n} {s:?}: read {} backends ({} distinct ids), set (addr,weight,healthy) equal={}", got.len(), st.distinct, got == want);
            check(&format!("exact set n={n} {s:?}"), got == want);
        }
    }

    println!("== swap semantics: a held inner fd is an immutable snapshot after update_service swaps the outer slot");
    let b1 = random_set(&mut rng, 0x0A90_0000, 16, 8080);
    dp.update_service(f.clone(), b1.clone()).await.unwrap();
    let old_id = rd.outer_id(&f).unwrap().unwrap();
    let old_fd = get_fd_by_id(old_id).unwrap();
    let b2 = random_set(&mut rng, 0x0A91_0000, 16, 8080);
    dp.update_service(f.clone(), b2.clone()).await.unwrap();
    let new_id = rd.outer_id(&f).unwrap().unwrap();
    println!("outer id before swap={old_id} after swap={new_id}");
    check("outer id changes on swap", old_id != new_id);
    let mut v = [0u8; 4];
    check("old inner fd still readable after swap (kernel refcount)", lookup_into(old_fd.as_raw_fd(), &0u32.to_ne_bytes(), &mut v).unwrap());
    drop(old_fd);
    check("old inner id freed once last fd dropped (GET_FD_BY_ID -> ENOENT)", matches!(get_fd_by_id(old_id), Err(ref e) if enoent(e)));

    println!("== replace smaller, withdraw, protocol independence, shared-endpoint caveat, IPv6");
    let small = random_set(&mut rng, 0x0AA0_0000, 3, 8080);
    dp.update_service(f.clone(), small.clone()).await.unwrap();
    check("shrink: exactly the new 3", rd.read(&f, Strat::Batch(INNER_CAP)).unwrap().0 == project(&small));
    dp.update_service(f.clone(), vec![]).await.unwrap();
    check("withdraw: empty, slot gone", rd.read(&f, Strat::Batch(INNER_CAP)).unwrap().0.is_empty() && rd.outer_id(&f).unwrap().is_none());
    let ft = frontend(Ipv4Addr::new(10, 96, 0, 2), 53, Proto::Tcp);
    let fu = frontend(Ipv4Addr::new(10, 96, 0, 2), 53, Proto::Udp);
    let bt = random_set(&mut rng, 0x0AB0_0000, 4, 5353);
    let bu = random_set(&mut rng, 0x0AB1_0000, 5, 5353);
    dp.update_service(ft.clone(), bt.clone()).await.unwrap();
    dp.update_service(fu.clone(), bu.clone()).await.unwrap();
    check("tcp/53 and udp/53 on one VIP are independent keys", rd.read(&ft, Strat::Batch(INNER_CAP)).unwrap().0 == project(&bt) && rd.read(&fu, Strat::Batch(INNER_CAP)).unwrap().0 == project(&bu));
    dp.update_service(fu.clone(), vec![]).await.unwrap();
    check("withdrawing udp/53 leaves tcp/53 intact", rd.read(&ft, Strat::Batch(INNER_CAP)).unwrap().0 == project(&bt) && rd.read(&fu, Strat::Batch(INNER_CAP)).unwrap().0.is_empty());
    // shared endpoint, same VIP, two ports: BACKEND_MAP is keyed by endpoint, so weight/healthy is last-writer-wins
    let fa = frontend(Ipv4Addr::new(10, 96, 0, 3), 80, Proto::Tcp);
    let fb = frontend(Ipv4Addr::new(10, 96, 0, 3), 443, Proto::Tcp);
    let ep = ip_at(0x0AC0_0000, 0);
    dp.update_service(fa.clone(), vec![backend(ep, 8080, 10, true)]).await.unwrap();
    dp.update_service(fb.clone(), vec![backend(ep, 8080, 99, false)]).await.unwrap();
    let ra = rd.read(&fa, Strat::Batch(INNER_CAP)).unwrap().0;
    println!("shared endpoint {ep}:8080 programmed w=10 healthy on :80, then w=99 unhealthy on :443; read(:80) = {ra:?}");
    println!("OBSERVATION shared-endpoint read(:80) reflects the LATER write (weight 99, unhealthy): {}", ra.iter().next().map(|p| p.weight == 99 && !p.healthy).unwrap());
    // IPv6 backend
    let v6 = Backend { alloc: SpiffeId::new("spiffe://overdrive.local/workload/spk/alloc/v6").unwrap(), addr: "[2001:db8::1]:80".parse().unwrap(), weight: 1, healthy: true };
    let before = rd.read(&ft, Strat::Batch(INNER_CAP)).unwrap().0;
    let r = dp.update_service(ft.clone(), vec![backend(ip_at(0x0AD0_0000, 0), 80, 1, true), v6]).await;
    println!("update_service with an IPv6 backend -> {:?}; read after == read before: {}", r.as_ref().map(|_| "Ok").map_err(|e| e.to_string()), rd.read(&ft, Strat::Batch(INNER_CAP)).unwrap().0 == before);
    println!("ALL CORRECTNESS CHECKS PASSED");
}

fn race(secs: u64) {
    let pin = pin_dir_arg();
    let dp = make_dp(&pin);
    let rd = Arc::new(Reader::new(&pin).unwrap());
    let f = frontend(Ipv4Addr::new(10, 96, 1, 1), 80, Proto::Tcp);
    // A and B share endpoints 8..16 with DIFFERENT weight/health, and each has 8 private endpoints,
    // so a mixed read can arise from BACKEND_MAP (updated in place) and from orphan cleanup (ENOENT).
    let mk = |w: u16, h: bool, from: u32, to: u32| -> Vec<Backend> { (from..to).map(|i| backend(ip_at(0x0AE0_0000, i), 8080, w + (i % 7) as u16, h)).collect() };
    let a: Vec<Backend> = mk(10, true, 0, 16);
    let b: Vec<Backend> = mk(500, false, 8, 24);
    let (pa, pb) = (project(&a), project(&b));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(dp.update_service(f.clone(), a.clone())).unwrap();

    for guarded in [false, true] {
        let lock = Arc::new(RwLock::new(()));
        let stop = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(AtomicU64::new(0));
        let c = || Arc::new(AtomicU64::new(0));
        let (n_a, n_b, n_torn, n_err_inner, n_err_backend, n_reads) = (c(), c(), c(), c(), c(), c());
        let mut hs = vec![];
        {
            let (dp, lock, stop, writes, f, a, b) = (dp.clone(), lock.clone(), stop.clone(), writes.clone(), f.clone(), a.clone(), b.clone());
            hs.push(std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let mut flip = false;
                while !stop.load(Ordering::Relaxed) {
                    let set = if flip { a.clone() } else { b.clone() };
                    flip = !flip;
                    if guarded { let _g = lock.write().unwrap(); rt.block_on(dp.update_service(f.clone(), set)).unwrap(); }
                    else { rt.block_on(dp.update_service(f.clone(), set)).unwrap(); }
                    writes.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for _ in 0..2 {
            let (rd, lock, stop, f, pa, pb) = (rd.clone(), lock.clone(), stop.clone(), f.clone(), pa.clone(), pb.clone());
            let (n_a, n_b, n_torn, ei, eb, nr) = (n_a.clone(), n_b.clone(), n_torn.clone(), n_err_inner.clone(), n_err_backend.clone(), n_reads.clone());
            hs.push(std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let r = if guarded { let _g = lock.read().unwrap(); rd.read(&f, Strat::Batch(INNER_CAP)) } else { rd.read(&f, Strat::Batch(INNER_CAP)) };
                    nr.fetch_add(1, Ordering::Relaxed);
                    match r {
                        Ok((s, _)) if s == pa => { n_a.fetch_add(1, Ordering::Relaxed); }
                        Ok((s, _)) if s == pb => { n_b.fetch_add(1, Ordering::Relaxed); }
                        Ok(_) => { n_torn.fetch_add(1, Ordering::Relaxed); }
                        Err(ReadErr::InnerGone(_)) => { ei.fetch_add(1, Ordering::Relaxed); }
                        Err(ReadErr::BackendMissing(_)) => { eb.fetch_add(1, Ordering::Relaxed); }
                        Err(ReadErr::Sys(e)) => panic!("sys error {e}"),
                    }
                }
            }));
        }
        std::thread::sleep(Duration::from_secs(secs));
        stop.store(true, Ordering::Relaxed);
        for h in hs { h.join().unwrap(); }
        println!("guard={} secs={secs} writes={} reads={} == A:{} B:{} TORN(neither A nor B):{} inner-gone(ENOENT on GET_FD_BY_ID):{} backend-missing(ENOENT on BACKEND_MAP):{}",
            if guarded { "spike-level RwLock around the whole update_service" } else { "NONE" },
            writes.load(Ordering::Relaxed), n_reads.load(Ordering::Relaxed), n_a.load(Ordering::Relaxed), n_b.load(Ordering::Relaxed), n_torn.load(Ordering::Relaxed), n_err_inner.load(Ordering::Relaxed), n_err_backend.load(Ordering::Relaxed));
    }
}

async fn restart() {
    let pin = pin_dir_arg();
    let mut rng = Lcg(7);
    let f = frontend(Ipv4Addr::new(10, 96, 2, 1), 80, Proto::Tcp);
    let b = random_set(&mut rng, 0x0AF0_0000, 16, 8080);
    {
        let dp = make_dp(&pin);
        let rd = Reader::new(&pin).unwrap();
        dp.update_service(f.clone(), b.clone()).await.unwrap();
        let (got, _) = rd.read(&f, Strat::Batch(INNER_CAP)).unwrap();
        println!("instance 1: programmed 16 backends; read {} backends; exact={}", got.len(), got == project(&b));
        println!("instance 1: SERVICE_MAP id={} BACKEND_MAP id={}", info_by_fd(rd.outer.as_raw_fd()).unwrap().id, rd.backend_id);
        check("instance 1 exact", got == project(&b));
    } // dp + reader dropped: the `serve` process exiting
    println!("maps named BACKEND_MAP after instance 1 dropped: {:?}", maps_named("BACKEND_MAP").unwrap().iter().map(|x| x.0).collect::<Vec<_>>());
    println!("pin path still present after drop: {}", pin.join("SERVICE_MAP").exists());
    let dp2 = make_dp(&pin);
    let rd2 = Reader::new(&pin).unwrap();
    let id2 = info_by_fd(rd2.outer.as_raw_fd()).unwrap().id;
    let (got2, _) = rd2.read(&f, Strat::Batch(INNER_CAP)).unwrap();
    println!("instance 2 over the SAME pin dir: SERVICE_MAP id={id2} BACKEND_MAP id={} read -> {} backends, outer slot present={}", rd2.backend_id, got2.len(), rd2.outer_id(&f).unwrap().is_some());
    check("fresh dataplane over same pin dir reads empty (serve restart)", got2.is_empty());
    dp2.update_service(f.clone(), b.clone()).await.unwrap();
    check("reprogramming after restart reads exact again", rd2.read(&f, Strat::Batch(INNER_CAP)).unwrap().0 == project(&b));
    drop(rd2);
    drop(dp2);
    // distinct pin dir = what a node reboot looks like (empty bpffs, no kernel objects)
    let pin_b = PathBuf::from(format!("{}-fresh", pin.display()));
    let dp3 = make_dp(&pin_b);
    let rd3 = Reader::new(&pin_b).unwrap();
    check("brand-new bpffs directory (reboot analogue) reads empty", rd3.read(&f, Strat::Batch(INNER_CAP)).unwrap().0.is_empty());
    drop(rd3);
    drop(dp3);
    let _ = std::fs::remove_dir_all(&pin_b);
}

async fn cost() {
    let pin = pin_dir_arg();
    let dp = make_dp(&pin);
    let rd = Reader::new(&pin).unwrap();
    let mut rng = Lcg(99);
    let strats = [("single-lookups(16381 syscalls)", Strat::Single), ("batch(16381)", Strat::Batch(INNER_CAP)), ("batch(4096)", Strat::Batch(4096)), ("batch(1024)", Strat::Batch(1024))];

    println!("== per-frontend read cost by backend count (one frontend programmed)");
    for n in [1u32, 16, 256] {
        let f = frontend(Ipv4Addr::new(10, 97, 0, 1), 80, Proto::Tcp);
        let b = random_set(&mut rng, 0x0B00_0000, n, 8080);
        dp.update_service(f.clone(), b.clone()).await.unwrap();
        let want = project(&b);
        for (name, s) in strats {
            let iters = if matches!(s, Strat::Single) { 100 } else { 300 };
            for _ in 0..5 { rd.read(&f, s).unwrap(); }
            let mut tot = vec![]; let (mut so, mut sg, mut si, mut sb) = (vec![], vec![], vec![], vec![]);
            for _ in 0..iters {
                let t = Instant::now(); let (got, st) = rd.read(&f, s).unwrap(); tot.push(t.elapsed());
                assert!(got == want);
                so.push(st.outer); sg.push(st.getfd); si.push(st.inner); sb.push(st.backend);
            }
            println!("backends={n:>3} {name:<31} total: {}", stats(tot));
            println!("                                        step p50: outer {} | getfd {} | inner-scan {} | backend-lookups {}", p50(so), p50(sg), p50(si), p50(sb));
        }
        dp.update_service(f.clone(), vec![]).await.unwrap();
    }

    println!("== per-frontend read cost with many frontends programmed (outer map occupancy)");
    for (nf, nb) in [(100u32, 16u32), (1000, 16), (100, 256)] {
        let t = Instant::now();
        let fronts: Vec<ServiceFrontend> = (0..nf).map(|i| frontend(Ipv4Addr::from(0x0A61_0000 + i), 80, Proto::Tcp)).collect();
        let mut want = vec![];
        for (i, f) in fronts.iter().enumerate() {
            let b = random_set(&mut rng, 0x0B80_0000 + (i as u32) * 256, nb, 8080);
            dp.update_service(f.clone(), b.clone()).await.unwrap();
            want.push(project(&b));
        }
        println!("programmed {nf} frontends x {nb} backends in {:.1}s", t.elapsed().as_secs_f64());
        let bi = info_by_fd(rd.backend.as_raw_fd()).unwrap();
        println!("BACKEND_MAP max_entries={} ; SERVICE_MAP outer max_entries={}", bi.max_entries, info_by_fd(rd.outer.as_raw_fd()).unwrap().max_entries);
        for (name, s) in [strats[0], strats[1]] {
            let mut per = vec![];
            let sweep = Instant::now();
            for (i, f) in fronts.iter().enumerate() {
                let t = Instant::now(); let (got, _) = rd.read(f, s).unwrap(); per.push(t.elapsed());
                assert!(got == want[i], "frontend {i} mismatch");
            }
            let tot = sweep.elapsed();
            println!("frontends={nf:>4} backends={nb:>3} {name:<31} per-frontend {} | full sweep of all {nf} = {:.1} ms", stats(per), tot.as_secs_f64() * 1e3);
        }
        for f in &fronts { dp.update_service(f.clone(), vec![]).await.unwrap(); }
    }
}

// ---------------------------------------------------------------- compare: read mechanisms
// Standalone micro-benchmark over maps this probe creates itself (same shapes as the dataplane's
// SERVICE_MAP: HASH_OF_MAPS key 8 / value 4, inner ARRAY u32->u32 of the Maglev table size).
// Variants: Baseline (batch scan of the 16,381-slot table), Fingerprint (two extra slots
// 16381/16382 hold a 64-bit fingerprint written before the outer swap; read = 2 lookups),
// Mmap (inner arrays BPF_F_MMAPABLE; read = mmap + scan).
const BPF_MAP_CREATE: i64 = 0;
const BPF_MAP_UPDATE_ELEM: i64 = 2;
const BPF_F_MMAPABLE: u32 = 1 << 10;
const T_ARRAY: u32 = 2;
const T_HOM: u32 = 13;

fn map_create(ty: u32, ks: u32, vs: u32, max: u32, flags: u32, inner: Option<i32>) -> io::Result<OwnedFd> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, ty); put32(&mut a, 4, ks); put32(&mut a, 8, vs); put32(&mut a, 12, max);
    put32(&mut a, 16, flags); put32(&mut a, 20, inner.unwrap_or(0) as u32);
    let fd = sys(BPF_MAP_CREATE, &mut a)?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
fn update(fd: i32, key: &[u8], val: &[u8]) -> io::Result<()> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, fd as u32); put64(&mut a, 8, key.as_ptr() as u64); put64(&mut a, 16, val.as_ptr() as u64);
    sys(BPF_MAP_UPDATE_ELEM, &mut a).map(|_| ())
}
fn get_fd_by_id_flags(id: u32, flags: u32) -> io::Result<OwnedFd> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, id); put32(&mut a, 8, flags);
    let fd = sys(BPF_MAP_GET_FD_BY_ID, &mut a)?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}


fn thread_cpu() -> Duration {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
    let tv = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    tv(ru.ru_utime) + tv(ru.ru_stime)
}
fn memlock(fd: i32) -> u64 {
    std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).unwrap().lines()
        .find_map(|l| l.strip_prefix("memlock:").map(|v| v.trim().parse().unwrap())).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Var { Baseline, Fingerprint, Mmap }

fn pct(mut v: Vec<Duration>) -> (Duration, Duration) {
    v.sort();
    (v[v.len() / 2], v[(v.len() * 99 / 100).min(v.len() - 1)])
}

fn compare() {
    let frontends: u32 = 1000;
    let backends: u32 = 16;
    for var in [Var::Baseline, Var::Fingerprint, Var::Mmap] {
        let cap = if var == Var::Fingerprint { INNER_CAP + 2 } else { INNER_CAP };
        let flags = if var == Var::Mmap { BPF_F_MMAPABLE } else { 0 };
        let proto = map_create(T_ARRAY, 4, 4, cap, flags, None).expect("proto");
        let outer = map_create(T_HOM, 8, 4, 4096, 0, Some(proto.as_raw_fd())).expect("outer");
        let mut inners = vec![];
        let mut create_times = vec![];
        let apply_cpu0 = thread_cpu();
        for f in 0..frontends {
            let t = Instant::now();
            let inner = map_create(T_ARRAY, 4, 4, cap, flags, None).expect("inner");
            for slot in 0..INNER_CAP {
                let id = 1 + f * 64 + (slot % backends);
                update(inner.as_raw_fd(), &slot.to_ne_bytes(), &id.to_ne_bytes()).unwrap();
            }
            if var == Var::Fingerprint {
                let fp: u64 = 0x9E37_79B9_7F4A_7C15 ^ u64::from(f);
                update(inner.as_raw_fd(), &INNER_CAP.to_ne_bytes(), &(fp as u32).to_ne_bytes()).unwrap();
                update(inner.as_raw_fd(), &(INNER_CAP + 1).to_ne_bytes(), &((fp >> 32) as u32).to_ne_bytes()).unwrap();
            }
            let key = u64::from(f).to_ne_bytes();
            let ifd = inner.as_raw_fd() as u32;
            update(outer.as_raw_fd(), &key, &ifd.to_ne_bytes()).expect("outer update (inner meta must match prototype)");
            create_times.push(t.elapsed());
            inners.push(inner);
        }
        let apply_cpu = thread_cpu() - apply_cpu0;
        let inner_mem = memlock(inners[0].as_raw_fd());
        let outer_mem = memlock(outer.as_raw_fd());
        let (cp50, cp99) = pct(create_times);
        let mut reads = vec![];
        let mut ok = true;
        let sweep = Instant::now();
        let read_cpu0 = thread_cpu();
        for f in 0..frontends {
            let t = Instant::now();
            let key = u64::from(f).to_ne_bytes();
            let mut idb = [0u8; 4];
            assert!(lookup_into(outer.as_raw_fd(), &key, &mut idb).unwrap());
            let id = u32::from_ne_bytes(idb);
            match var {
                Var::Baseline => {
                    let fd = get_fd_by_id(id).unwrap();
                    let mut keys = vec![0u32; INNER_CAP as usize];
                    let mut vals = vec![0u32; INNER_CAP as usize];
                    let mut ob = [0u8; 4];
                    let (n, _) = batch_call(fd.as_raw_fd(), None, &mut ob, &mut keys, &mut vals).unwrap();
                    let d: BTreeSet<u32> = vals[..n as usize].iter().copied().collect();
                    reads.push(t.elapsed());
                    ok &= d.len() == backends as usize;
                }
                Var::Fingerprint => {
                    let fd = get_fd_by_id(id).unwrap();
                    let (mut lo, mut hi) = ([0u8; 4], [0u8; 4]);
                    lookup_into(fd.as_raw_fd(), &INNER_CAP.to_ne_bytes(), &mut lo).unwrap();
                    lookup_into(fd.as_raw_fd(), &(INNER_CAP + 1).to_ne_bytes(), &mut hi).unwrap();
                    let fp = u64::from(u32::from_ne_bytes(lo)) | (u64::from(u32::from_ne_bytes(hi)) << 32);
                    reads.push(t.elapsed());
                    ok &= fp == 0x9E37_79B9_7F4A_7C15 ^ u64::from(f);
                }
                Var::Mmap => {
                    let fd = get_fd_by_id_flags(id, 0).unwrap(); // a BPF_F_RDONLY fd drops VM_SHARED on MAP_SHARED, which bpf_map_mmap refuses
                    // mmapable array elements are value_size rounded up to 8 bytes.
                    let len = (INNER_CAP as usize * 8 + 4095) & !4095;
                    let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
                    if p == libc::MAP_FAILED { let e = io::Error::last_os_error(); let mut info=[0u8;128]; let mut a=[0u8;128]; put32(&mut a,0,fd.as_raw_fd() as u32); put32(&mut a,4,128); put64(&mut a,8,info.as_mut_ptr() as u64); sys(BPF_OBJ_GET_INFO_BY_FD,&mut a).unwrap(); let mf=u32::from_ne_bytes(info[20..24].try_into().unwrap()); let direct = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, inners[f as usize].as_raw_fd(), 0) }; panic!("mmap via id: {e}; map_flags={mf:#x}; direct mmap on creating fd ok={}", direct != libc::MAP_FAILED); }
                    let base = p as *const u32;
                    let mut d = BTreeSet::new();
                    for slot in 0..INNER_CAP as usize { d.insert(unsafe { *base.add(slot * 2) }); }
                    unsafe { libc::munmap(p, len) };
                    reads.push(t.elapsed());
                    ok &= d.len() == backends as usize;
                }
            }
        }
        let read_cpu = thread_cpu() - read_cpu0;
        let sweep = sweep.elapsed();
        let (p50, p99) = pct(reads);
        println!("{var:?}: frontends={frontends} backends={backends} read p50={p50:?} p99={p99:?} sweep(all {frontends})={sweep:?} correct={ok} | apply(create+fill+swap) p50={cp50:?} p99={cp99:?}");
        println!("{var:?}:   CPU (thread user+sys): read sweep={read_cpu:?} ({:?}/read) | apply all={apply_cpu:?} ({:?}/apply)", read_cpu / frontends, apply_cpu / frontends);
        println!("{var:?}:   kernel memlock: inner map={inner_mem} B each, x{frontends} live = {} KiB; outer map={outer_mem} B", inner_mem * u64::from(frontends) / 1024);
    }
}
