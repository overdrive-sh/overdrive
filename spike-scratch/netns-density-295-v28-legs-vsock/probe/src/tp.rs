//! Userspace half of the kernel flag classifier (EOF class "tp"): loads the embedded tracepoint
//! ELF, drains its ringbuf at EOF time, and remembers per flow the FIRST SHUTDOWN/RST the peer sent.
//! First SHUTDOWN with the SEND bit only (0x2) = peer half-closed; 0x3 first (or RST first) = the
//! peer closed/aborted with its direction still open.
use aya::{maps::RingBuf, programs::TracePoint, Ebpf};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

static OBJ: &[u8] = include_bytes!("../tp-bpf.o");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum First { Half, Full, Rst }

struct Tp {
    _bpf: Ebpf,
    ring: RingBuf<aya::maps::MapData>,
    // (peer_cid, peer_port, local_port) -> first op seen from the peer
    first: BTreeMap<(u32, u32, u32), (First, u32)>,
    records: u64,
}
static TP: OnceLock<Mutex<Tp>> = OnceLock::new();

pub fn init() -> Result<(), String> {
    let mut bpf = Ebpf::load(&OBJ.to_vec()).map_err(|e| format!("load: {e:?}"))?;
    let p: &mut TracePoint = bpf.program_mut("vsock_recv").ok_or("no prog")?.try_into().map_err(|e| format!("{e:?}"))?;
    p.load().map_err(|e| format!("prog load: {e:?}"))?;
    p.attach("vsock", "virtio_transport_recv_pkt").map_err(|e| format!("attach: {e:?}"))?;
    let ring = RingBuf::try_from(bpf.take_map("SHUTRB").ok_or("no ring")?).map_err(|e| format!("ring: {e:?}"))?;
    let _ = TP.set(Mutex::new(Tp { _bpf: bpf, ring, first: BTreeMap::new(), records: 0 }));
    Ok(())
}

fn u32_at(b: &[u8], o: usize) -> u32 { u32::from_ne_bytes(b[o..o + 4].try_into().unwrap()) }

/// Drain the ring, then return the first op the peer sent on this flow (None = nothing recorded).
pub fn first_from_peer(peer_cid: u32, peer_port: u32, local_port: u32) -> Option<(First, u32)> {
    let mut guard = TP.get()?.lock().unwrap();
    let g = &mut *guard;
    let (ring, first, records) = (&mut g.ring, &mut g.first, &mut g.records);
    while let Some(item) = ring.next() {
        if item.len() < 32 { continue; }
        let (src_cid, src_port, dst_port, op, flags) = (u32_at(&item, 8), u32_at(&item, 12), u32_at(&item, 20), u32_at(&item, 24), u32_at(&item, 28));
        *records += 1;
        let k = (src_cid, src_port, dst_port);
        let f = if op == 3 { First::Rst } else if flags & 1 != 0 { First::Full } else { First::Half };
        first.entry(k).or_insert((f, flags));
    }
    first.get(&(peer_cid, peer_port, local_port)).copied()
}
pub fn forget(peer_cid: u32, peer_port: u32, local_port: u32) {
    if let Some(t) = TP.get() { t.lock().unwrap().first.remove(&(peer_cid, peer_port, local_port)); }
}
pub fn records() -> u64 { TP.get().map(|t| t.lock().unwrap().records).unwrap_or(0) }
