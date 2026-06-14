//! Production mTLS intercept-install surface.
//!
//! Loads the shared `overdrive_bpf.o`, owns the `cgroup_connect4_mtls` program
//! handle + the `MTLS_REDIRECT_DEST` typed map, and exposes per-alloc attach +
//! per-destination redirect programming. SEPARATE from
//! [`EbpfDataplane`](crate::EbpfDataplane); NOT a
//! [`Dataplane`](overdrive_core::traits::Dataplane) method (D-MTLS-17 item 1).
//!
//! The `cgroup_connect4_mtls` program is **load-once, attach-per-alloc**: the BPF
//! object is loaded once (the program FD lives for the process lifetime of the
//! owning `MtlsDataplane`), and a fresh `CgroupSockAddrLink` is taken per
//! allocation against that alloc's own `.scope` cgroup. Per-alloc attach (not the
//! global `workloads.slice` the service program uses) IS the F5 exemption made
//! structural — the program sees only THIS workload's `connect()`s, never the
//! agent's own leg-B dial (which runs on the host, outside any workload scope).
//!
//! The `MTLS_REDIRECT_DEST` map is a `BPF_MAP_TYPE_HASH` keyed on host-order
//! [`MtlsDestKey`] → [`MtlsAddrPort`] PODs (8-byte mirrors of the kernel-side
//! structs in `overdrive-bpf::maps::mtls_redirect_dest`). Userspace stores
//! host-order bytes; the kernel-side `cgroup_connect4_mtls` program converts to
//! network byte order on the rewrite (`.claude/rules/development.md` §
//! "Userspace map insertion → Endianness lockstep"). The map is native aya
//! (`bpf.take_map`); no HoM `pinning = ByName` dance.

use std::net::SocketAddrV4;
use std::path::Path;

use aya::maps::HashMap as AyaHashMap;
use aya::programs::{CgroupAttachMode, CgroupSockAddr};
use aya::{Ebpf, EbpfLoader};

use crate::maps::ServiceKey;
use crate::maps::hash_of_maps::HashOfMapsHandle;
use crate::{
    OVERDRIVE_BPF_OBJ, SERVICE_MAP_INNER_CAPACITY, SERVICE_MAP_NAME, SERVICE_MAP_OUTER_CAPACITY,
};

/// Cause-distinct failure modes for the production mTLS intercept-install.
///
/// Typed (`thiserror`), no `Internal(String)` catch-all; `Display` names the
/// kernel / privilege remediation (`.claude/rules/development.md` § Errors).
#[derive(Debug, thiserror::Error)]
pub enum MtlsDataplaneError {
    /// The shared BPF object failed to load, or `cgroup_connect4_mtls` /
    /// `MTLS_REDIRECT_DEST` was absent from it (a build/embed regression).
    #[error("mTLS BPF load failed: {reason}")]
    Load {
        /// The underlying loader / recovery failure.
        reason: String,
    },
    /// `cgroup_connect4_mtls.load()` (the verifier pass) was rejected.
    #[error("cgroup_connect4_mtls verifier load failed: {reason}")]
    ProgramLoad {
        /// The verifier rejection detail.
        reason: String,
    },
    /// Per-alloc attach to the allocation's `.scope` cgroup failed (the scope dir
    /// is missing, or `CAP_BPF` / `CAP_NET_ADMIN` is absent).
    #[error("cgroup_connect4_mtls attach to {scope} failed: {source}")]
    Attach {
        /// The `.scope` cgroup path the attach targeted.
        scope: std::path::PathBuf,
        /// The underlying attach syscall failure.
        #[source]
        source: std::io::Error,
    },
    /// `MTLS_REDIRECT_DEST` update / delete syscall failed.
    #[error("MTLS_REDIRECT_DEST {op} failed: {source}")]
    MapProgram {
        /// The operation that failed (`"insert"` / `"remove"`).
        op: &'static str,
        /// The underlying map syscall failure.
        #[source]
        source: std::io::Error,
    },
}

/// Result alias used throughout the module.
pub type Result<T, E = MtlsDataplaneError> = std::result::Result<T, E>;

/// `MTLS_REDIRECT_DEST` key — the real-peer destination the workload aimed at,
/// host-order. 8-byte `#[repr(C)]` POD mirroring the kernel-side `MtlsDestKey` in
/// `overdrive-bpf::maps::mtls_redirect_dest`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "`_pad` is a load-bearing wire field — it documents the always-zero \
              padding the kernel hashes and mirrors the kernel-side `MtlsDestKey` \
              POD name byte-for-byte; renaming would desync the two structs"
)]
pub struct MtlsDestKey {
    /// Real-peer IPv4, host-order (`u32::from(Ipv4Addr)`).
    pub ip_host: u32,
    /// Real-peer port, host-order.
    pub port_host: u16,
    /// Padding to 8-byte alignment. Always zero — the kernel hashes the full key
    /// bytes, so an uninitialised pad would split logically-equal keys.
    pub _pad: u16,
}

// SAFETY: 8-byte `#[repr(C)]` POD with no padding-derived invariants beyond the
// explicit zero `_pad`. Matches the kernel-side `MtlsDestKey` byte layout.
unsafe impl aya::Pod for MtlsDestKey {}

/// Compile-time guard: the key MUST stay 8 bytes to match the kernel-side struct
/// — a drift fails the build here, not silently at the next mis-keyed lookup.
const _: () = assert!(core::mem::size_of::<MtlsDestKey>() == 8);

/// `MTLS_REDIRECT_DEST` value — the agent leg-F listener the connect is rewritten
/// to, host-order. 8-byte `#[repr(C)]` POD mirroring the kernel-side
/// `MtlsAddrPort`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "`_pad` is a load-bearing wire field mirroring the kernel-side \
              `MtlsAddrPort` POD name byte-for-byte; renaming would desync the two"
)]
pub struct MtlsAddrPort {
    /// Agent leg-F listener IPv4, host-order. The kernel program writes
    /// `bpf_sock_addr.user_ip4 = ip_host.to_be()`.
    pub ip_host: u32,
    /// Agent leg-F listener port, host-order. The kernel program writes
    /// `user_port = u32::from(port_host.to_be())`.
    pub port_host: u16,
    /// Padding for 8-byte alignment. Always zero.
    pub _pad: u16,
}

// SAFETY: 8-byte `#[repr(C)]` POD. Matches the kernel-side `MtlsAddrPort` layout.
unsafe impl aya::Pod for MtlsAddrPort {}

/// Compile-time guard: the value MUST stay 8 bytes to match the kernel-side
/// struct.
const _: () = assert!(core::mem::size_of::<MtlsAddrPort>() == 8);

impl MtlsDestKey {
    /// Host-order key from a real-peer socket address. `u32::from(Ipv4Addr)` is
    /// host-order on every supported arch (the endianness-lockstep boundary —
    /// userspace stores host-order, the kernel program converts to NBO on the
    /// rewrite).
    fn from_peer(peer: SocketAddrV4) -> Self {
        Self { ip_host: u32::from(*peer.ip()), port_host: peer.port(), _pad: 0 }
    }
}

impl MtlsAddrPort {
    /// Host-order value from the agent leg-F listener address.
    fn from_leg_f(leg_f: SocketAddrV4) -> Self {
        Self { ip_host: u32::from(*leg_f.ip()), port_host: leg_f.port(), _pad: 0 }
    }
}

/// Kernel-side program name for the OUTBOUND mTLS intercept.
const MTLS_CONNECT4_PROG: &str = "cgroup_connect4_mtls";

/// Kernel-side map name for the OUTBOUND redirect destination table.
const MTLS_REDIRECT_DEST_MAP: &str = "MTLS_REDIRECT_DEST";

/// The production mTLS intercept-install surface. Constructed ONCE per process
/// (load-once); [`attach_alloc`](Self::attach_alloc) is called per-allocation
/// (attach-per-alloc).
pub struct MtlsDataplane {
    /// The loaded BPF object — owns the `cgroup_connect4_mtls` program FD (the
    /// verifier-loaded program lives as long as this `Ebpf` value) and is the
    /// source of `program_mut` for per-alloc attach.
    bpf: Ebpf,
    /// The `MTLS_REDIRECT_DEST` typed map handle, recovered once at load. Behind a
    /// `Mutex` so `program_redirect` / `unprogram_redirect` take `&self`.
    redirect_dest: parking_lot::Mutex<AyaHashMap<aya::maps::MapData, MtlsDestKey, MtlsAddrPort>>,
}

impl MtlsDataplane {
    /// Load the shared `overdrive_bpf.o`, recover the `cgroup_connect4_mtls`
    /// program handle and the `MTLS_REDIRECT_DEST` typed map, and run the
    /// program's verifier load ONCE. Mirrors `EbpfDataplane::new_with_pin_dir`'s
    /// recover-from-the-loaded-ELF shape. No attach yet — attach is per-alloc.
    ///
    /// `pin_dir` is the bpffs pin directory for the shared object's pinned service
    /// HoM (the `pinning = ByName` SERVICE_MAP). The mTLS map / program need no
    /// pin of their own.
    ///
    /// # Errors
    ///
    /// [`MtlsDataplaneError::Load`] if the shared object fails to load or
    /// `cgroup_connect4_mtls` / `MTLS_REDIRECT_DEST` is absent;
    /// [`MtlsDataplaneError::ProgramLoad`] if the verifier rejects the program.
    pub fn load(pin_dir: &Path) -> Result<Self> {
        // The shared `overdrive_bpf.o` carries the phase-2 SERVICE_MAP HASH_OF_MAPS,
        // which aya 0.13.x cannot create from the ELF alone. Pre-pin it by name into
        // `pin_dir` and load with `map_pin_path` so aya reuses the pinned outer FD
        // (`.claude/rules/development.md` § "Sharing the outer HoM … `pinning =
        // ByName`"). Mirrors `EbpfDataplane::new_with_pin_dir` (lib.rs:446-501).
        let pin_path = pin_dir.join(SERVICE_MAP_NAME);
        let _ = std::fs::remove_file(&pin_path); // clear any stale pin
        let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
            SERVICE_MAP_NAME,
            SERVICE_MAP_OUTER_CAPACITY,
            SERVICE_MAP_INNER_CAPACITY,
            pin_dir,
        )
        .map_err(|e| MtlsDataplaneError::Load { reason: format!("SERVICE_MAP pin: {e}") })?;
        // Keep the pin alive for the process lifetime — the loaded ELF holds the
        // pinned outer FD by name; dropping the handle would unpin it. The pinned
        // outer map outlives this handle (bpffs pins persist), so leaking the userspace
        // handle is the canonical shape (mirrors `load_workload_bpf` in the test glue).
        std::mem::forget(service_map);

        // Materialise the embedded object to a temp file (NOT under `pin_dir`, which
        // is a bpffs mount that rejects regular file writes), then load + remove.
        let bpf_temp_path =
            std::env::temp_dir().join(format!("overdrive_bpf_mtls-{}.o", std::process::id()));
        std::fs::write(&bpf_temp_path, OVERDRIVE_BPF_OBJ).map_err(|e| {
            MtlsDataplaneError::Load {
                reason: format!("write embedded BPF object to {}: {e}", bpf_temp_path.display()),
            }
        })?;
        let loaded = EbpfLoader::new()
            .map_pin_path(pin_dir)
            // aya 0.13.x has no HASH_OF_MAPS variant; SERVICE_MAP surfaces as
            // `Map::Unsupported`. We own it via the pinned `HashOfMapsHandle`, not
            // `take_map`, so tolerate the unsupported variant.
            .allow_unsupported_maps()
            .load_file(&bpf_temp_path)
            .map_err(|e| MtlsDataplaneError::Load { reason: format!("aya load: {e}") });
        let _ = std::fs::remove_file(&bpf_temp_path);
        let mut bpf = loaded?;

        // Recover the MTLS_REDIRECT_DEST typed map (native aya HASH; no HoM dance).
        let redirect_dest = AyaHashMap::<_, MtlsDestKey, MtlsAddrPort>::try_from(
            bpf.take_map(MTLS_REDIRECT_DEST_MAP).ok_or_else(|| MtlsDataplaneError::Load {
                reason: format!("{MTLS_REDIRECT_DEST_MAP} not found in BPF object"),
            })?,
        )
        .map_err(|e| MtlsDataplaneError::Load {
            reason: format!("{MTLS_REDIRECT_DEST_MAP} try_from: {e}"),
        })?;

        // Run the cgroup_connect4_mtls verifier load ONCE (program FD lives for the
        // process). attach is per-alloc, NOT here. aya recovers the attach type from
        // the kernel-side `link_section = "cgroup/connect4"` the
        // `#[cgroup_sock_addr(connect4)]` macro emits.
        let prog: &mut CgroupSockAddr = bpf
            .program_mut(MTLS_CONNECT4_PROG)
            .ok_or_else(|| MtlsDataplaneError::Load {
                reason: format!("{MTLS_CONNECT4_PROG} program not found in BPF object"),
            })?
            .try_into()
            .map_err(|e| MtlsDataplaneError::Load {
                reason: format!("{MTLS_CONNECT4_PROG} program type: {e}"),
            })?;
        prog.load().map_err(|e| MtlsDataplaneError::ProgramLoad { reason: format!("{e}") })?;

        Ok(Self { bpf, redirect_dest: parking_lot::Mutex::new(redirect_dest) })
    }

    /// Attach `cgroup_connect4_mtls` to ONE allocation's own `.scope` cgroup (the
    /// F5-exempt per-workload subtree — NOT the global `workloads.slice` ancestor
    /// the service program uses). Returns the owned link; the worker holds it
    /// per-alloc and drops it on teardown to detach.
    ///
    /// # Errors
    ///
    /// [`MtlsDataplaneError::Load`] if the program handle cannot be recovered;
    /// [`MtlsDataplaneError::Attach`] if the `.scope` cgroup cannot be opened or
    /// the attach syscall fails.
    pub fn attach_alloc(&mut self, alloc_scope: &Path) -> Result<MtlsCgroupLink> {
        // Open the allocation's OWN `.scope` cgroup (cgroup v2) — the F5-exempt
        // per-workload subtree, NOT the global `workloads.slice` ancestor the service
        // program attaches to. aya passes this fd to `bpf_link_create(LinkTarget::Fd)`.
        let cgroup_file = std::fs::File::open(alloc_scope).map_err(|e| {
            MtlsDataplaneError::Attach { scope: alloc_scope.to_path_buf(), source: e }
        })?;

        let prog: &mut CgroupSockAddr = self
            .bpf
            .program_mut(MTLS_CONNECT4_PROG)
            .ok_or_else(|| MtlsDataplaneError::Load {
                reason: format!("{MTLS_CONNECT4_PROG} program not found in BPF object"),
            })?
            .try_into()
            .map_err(|e| MtlsDataplaneError::Load {
                reason: format!("{MTLS_CONNECT4_PROG} program type: {e}"),
            })?;

        // The program was verifier-loaded once in `load()`. Attach to THIS alloc's
        // scope; on failure surface the typed `Attach` variant (the attach syscall
        // error carries the privilege / missing-scope cause).
        let link_id = prog.attach(&cgroup_file, CgroupAttachMode::Single).map_err(|e| {
            MtlsDataplaneError::Attach {
                scope: alloc_scope.to_path_buf(),
                source: std::io::Error::other(format!("{e}")),
            }
        })?;
        let link = prog.take_link(link_id).map_err(|e| MtlsDataplaneError::Attach {
            scope: alloc_scope.to_path_buf(),
            source: std::io::Error::other(format!("take_link: {e}")),
        })?;

        Ok(MtlsCgroupLink { _link: link })
    }

    /// Program `MTLS_REDIRECT_DEST[real_peer] = leg_f_listener` (host-order keys;
    /// the kernel program converts to NBO on the rewrite). Called by the worker
    /// BEFORE the workload connects, so the workload's `connect(real_peer)` is
    /// transparently rewritten to the agent's leg-F listener. Idempotent overwrite
    /// (re-programming the same peer replaces the leg-F target).
    ///
    /// # Errors
    ///
    /// [`MtlsDataplaneError::MapProgram`] if the map update syscall fails.
    pub fn program_redirect(&self, real_peer: SocketAddrV4, leg_f: SocketAddrV4) -> Result<()> {
        let key = MtlsDestKey::from_peer(real_peer);
        let val = MtlsAddrPort::from_leg_f(leg_f);
        // `BPF_ANY` (flags = 0) — insert-or-overwrite, the idempotent-overwrite
        // contract. Userspace stores HOST-order; the kernel program converts to NBO
        // on the rewrite (endianness lockstep).
        self.redirect_dest
            .lock()
            .insert(key, val, 0)
            .map_err(|e| MtlsDataplaneError::MapProgram { op: "insert", source: map_io_error(e) })
    }

    /// Remove the `MTLS_REDIRECT_DEST[real_peer]` entry (on alloc teardown).
    /// Absent key → `Ok` (idempotent remove).
    ///
    /// # Errors
    ///
    /// [`MtlsDataplaneError::MapProgram`] if the map delete syscall fails for a
    /// reason other than the key being absent.
    pub fn unprogram_redirect(&self, real_peer: SocketAddrV4) -> Result<()> {
        let key = MtlsDestKey::from_peer(real_peer);
        let removed = self.redirect_dest.lock().remove(&key);
        match removed {
            Ok(()) => Ok(()),
            // Absent key → Ok (idempotent remove). The kernel returns ENOENT when
            // deleting a key that is not present; every OTHER io error is a real
            // syscall failure surfaced as `MapProgram`.
            Err(e) => {
                let io = map_io_error(e);
                if io.raw_os_error() == Some(libc::ENOENT) {
                    Ok(())
                } else {
                    Err(MtlsDataplaneError::MapProgram { op: "remove", source: io })
                }
            }
        }
    }
}

/// Project an aya `MapError` onto an `io::Error` for the typed `MapProgram`
/// variant. `SyscallError` carries the originating `io_error` (the cause-distinct
/// signal — `ENOENT` for an absent key, `EPERM`/`EFAULT` for real failures);
/// non-syscall variants are flattened to an `Other` io error preserving the
/// `Display`.
fn map_io_error(e: aya::maps::MapError) -> std::io::Error {
    match e {
        aya::maps::MapError::SyscallError(syscall) => syscall.io_error,
        other => std::io::Error::other(format!("{other}")),
    }
}

/// RAII owner of one allocation's `cgroup_connect4_mtls` attach link. `Drop`
/// detaches the program from that alloc's `.scope`. Held by the worker per-alloc.
pub struct MtlsCgroupLink {
    /// The owned aya cgroup attach link; `Drop` detaches.
    _link: aya::programs::cgroup_sock_addr::CgroupSockAddrLink,
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use proptest::prelude::*;

    use super::{MtlsAddrPort, MtlsDestKey};

    /// The exact 8-byte host-order wire layout the kernel hashes: `ip_host` (4
    /// host-order bytes) ++ `port_host` (2 host-order bytes) ++ `_pad = 0` (2
    /// bytes). Asserting the full layout subsumes the pad-zero invariant and pins
    /// the byte order the `cgroup_connect4_mtls` lookup keys on — a mutation that
    /// flips an endianness, transposes ip/port, or leaves the pad nonzero is killed
    /// because the byte sequence no longer matches.
    fn expected_wire_bytes(ip: u32, port: u16) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&ip.to_ne_bytes());
        bytes[4..6].copy_from_slice(&port.to_ne_bytes());
        // bytes[6..8] stay zero — the `_pad` invariant.
        bytes
    }

    /// View an 8-byte `aya::Pod` (`#[repr(C)]`, no padding-derived invariants
    /// beyond the explicit zero `_pad`) as its raw bytes.
    fn pod_bytes<T: aya::Pod>(pod: &T) -> [u8; 8] {
        // SAFETY: both PODs are `#[repr(C)]` 8-byte `aya::Pod` types (asserted by
        // the `size_of == 8` consts above); reading them as 8 bytes is sound.
        let raw = unsafe { core::slice::from_raw_parts(core::ptr::from_ref(pod).cast::<u8>(), 8) };
        let mut out = [0u8; 8];
        out.copy_from_slice(raw);
        out
    }

    proptest! {
        /// `MtlsDestKey::from_peer` produces the exact host-order wire layout. The
        /// kernel does the NBO conversion on the rewrite (endianness-lockstep,
        /// development.md § "`bpf_sock_addr.user_port`"); userspace stores HOST-order.
        #[test]
        fn dest_key_is_host_order_wire_layout(ip in any::<u32>(), port in any::<u16>()) {
            let peer = SocketAddrV4::new(Ipv4Addr::from(ip), port);
            let key = MtlsDestKey::from_peer(peer);
            prop_assert_eq!(pod_bytes(&key), expected_wire_bytes(u32::from(*peer.ip()), peer.port()));
        }

        /// `MtlsAddrPort::from_leg_f` mirrors the key conversion for the leg-F addr.
        #[test]
        fn addr_port_is_host_order_wire_layout(ip in any::<u32>(), port in any::<u16>()) {
            let leg_f = SocketAddrV4::new(Ipv4Addr::from(ip), port);
            let val = MtlsAddrPort::from_leg_f(leg_f);
            prop_assert_eq!(pod_bytes(&val), expected_wire_bytes(u32::from(*leg_f.ip()), leg_f.port()));
        }
    }
}
