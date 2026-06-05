//! `overdrive-dataplane` — userspace BPF loader per ADR-0038.
//!
//! Owns [`EbpfDataplane`], the production binding of the
//! [`Dataplane`] port trait from `overdrive-core`. The kernel-side
//! object produced by `overdrive-bpf` is embedded at compile time via
//! `include_bytes!`; the loader attaches the configured XDP programs
//! to the requested interfaces.
//!
//! Phase 2.1 step 01-02 ships the loader skeleton. The three trait
//! methods (`update_policy`, `update_service`, `drain_flow_events`)
//! are stubbed pending #158 (`POLICY_MAP`), #24 (`SERVICE_MAP`), and
//! #31 (telemetry ringbuf) per `architecture.md` §7.

#![expect(
    clippy::doc_markdown,
    reason = "Phase 2.2 RED scaffolds in maglev/* and swap.rs; lints self-trip when scaffolds go GREEN. Strip when Slice 08 closes the last scaffold."
)]

pub mod maps;
pub mod swap;

// Allocator primitives — `BackendIdAllocator` (ADR-0046) lives here
// alongside `ServiceVipAllocator` + `VipRange` (ADR-0049 / step 01-01).
// `BackendIdAllocator` is the existing collision-free monotonic-counter
// allocator; `ServiceVipAllocator` is the concrete VIP-pool allocator
// keyed by service-spec digest. Both monotonic, both userspace-only;
// no shared abstraction by design — VIP allocation has its own
// (range, capacity, exhaustion) concerns that BackendId does not.
pub mod allocators;

// Orphan-GC sweep over `BACKEND_MAP` (step 4 of ADR-0040 § 2's
// 5-step swap orchestration).
pub mod gc;

// Direct `bpf(2)` syscall surface used where aya 0.13.x ships no
// typed wrappers (HASH_OF_MAPS construction + `BPF_PROG_TEST_RUN`).
pub mod sys;

use std::net::Ipv4Addr;

use async_trait::async_trait;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Embedded kernel-side BPF object. Produced by
/// `cargo xtask bpf-build` (step 02-01) and copied to the stable path
/// `target/bpf/overdrive_bpf.o`. The `build.rs` shim
/// (step 01-03) converts a missing artifact into a single-line
/// actionable error.
// `OVERDRIVE_BPF_OBJECT_PATH` is emitted by `build.rs` as a
// `cargo:rustc-env=` directive, resolved at build-script time against
// either the `OVERDRIVE_BPF_OBJECT` override (set by `cargo xtask
// mutants`) or the workspace-relative fallback. Single env-var entry
// point keeps the `include_bytes!` macro independent of how the path
// was computed — see `build.rs` module docstring for the override
// rationale and `xtask::mutants::bpf_object_env_override` for the
// wrapper-side mechanics.
const OVERDRIVE_BPF_OBJ: &[u8] = include_bytes!(env!("OVERDRIVE_BPF_OBJECT_PATH"));

/// Production bpffs pin directory for SERVICE_MAP and any future
/// HoM-shaped maps.
///
/// The kernel-side declaration carries `pinning = ByName`; aya's
/// loader joins this directory + the map name when resolving the
/// pre-pinned FD via `BPF_OBJ_GET`. See
/// `.claude/rules/development.md` § "Sharing the outer HoM between
/// userspace and the kernel-side ELF — `pinning = ByName`".
pub const DEFAULT_PIN_DIR: &str = "/sys/fs/bpf/overdrive";

/// SERVICE_MAP outer-map name. MUST match the `#[map]` `export_name`
/// emitted from `crates/overdrive-bpf/src/maps/service_map.rs` —
/// that name is what aya's loader uses to join `<pin_dir>/<name>`.
const SERVICE_MAP_NAME: &str = "SERVICE_MAP";

/// BACKEND_MAP name — regular HASH map; aya supports it natively
/// (no pin-by-name workaround needed).
const BACKEND_MAP_NAME: &str = "BACKEND_MAP";

/// REVERSE_NAT_MAP name — regular HASH map keyed on
/// `BackendKeyPod { ip_host, port_host, proto, _pad }` →
/// `VipPod { ip_host, port_host, _pad }`. aya supports HASH natively
/// (Slice 05-04: promoted from in-memory placeholder per
/// `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`).
const REVERSE_NAT_MAP_NAME: &str = "REVERSE_NAT_MAP";

/// LOCAL_BACKEND_MAP name per ADR-0053 § 1 — regular HASH map keyed
/// on `LocalServiceKey { vip_host, port_host, _pad }` →
/// `LocalBackendEntry { backend_ip_host, backend_port_host, _pad }`.
/// aya supports HASH natively.
const LOCAL_BACKEND_MAP_NAME: &str = "LOCAL_BACKEND_MAP";

/// REVERSE_LOCAL_MAP name per ADR-0053 rev 2026-06-05 (GH #200) —
/// regular HASH keyed on `ReverseLocalKeyPod { backend_ip_host,
/// backend_port_host, proto, _pad }` → `vip_host: u32`. The reply
/// store for the unconnected same-host UDP cgroup path; the
/// `cgroup_recvmsg4_service` program reads it. aya supports HASH
/// natively. DISTINCT from `REVERSE_NAT_MAP` (the XDP wire path).
const REVERSE_LOCAL_MAP_NAME: &str = "REVERSE_LOCAL_MAP";

/// Default cgroup attach path for `cgroup_connect4_service` per
/// ADR-0053 § 7.
///
/// Must be an ancestor of the control-plane process AND every
/// workload cgroup. Matches the slice that
/// `crates/overdrive-worker/src/cgroup_manager.rs` already manages.
pub const DEFAULT_CGROUP_ATTACH_PATH: &str = "/sys/fs/cgroup/overdrive.slice";

/// SERVICE_MAP outer-map capacity in services. 4096 per
/// architecture.md § 10. MUST match the kernel-side
/// `MAX_OUTER_ENTRIES` const in `service_map.rs` — kernel and
/// userspace see the same map (pin-by-name shares the FD), so the
/// capacities are consistent by definition.
const SERVICE_MAP_OUTER_CAPACITY: u32 = 4096;

/// SERVICE_MAP inner-ARRAY size in slots. Equals
/// [`overdrive_core::dataplane::MaglevTableSize::DEFAULT`].get() = 16_381
/// per architecture.md § 5 Q-Sig D6 / ADR-0041 — the Maglev table
/// size. **MUST** stay in lockstep with `INNER_TABLE_SIZE` in
/// `crates/overdrive-bpf/src/maps/service_map.rs` (kernel-side); a
/// drift between the two would silently misroute packets via slot
/// out-of-bounds reads (the kernel ARRAY map clamps to its declared
/// size; userspace populating slots beyond it is a no-op).
const SERVICE_MAP_INNER_CAPACITY: u32 = overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();

/// Production dataplane.
///
/// Loads `overdrive_bpf.o`, pre-creates and pre-pins the `SERVICE_MAP`
/// outer HASH_OF_MAPS so aya's loader reuses the FD via
/// `pinning = ByName` (per `.claude/rules/development.md`
/// § "Sharing the outer HoM between userspace and the kernel-side
/// ELF — `pinning = ByName`"), and attaches the configured XDP
/// program to the requested interface.
pub struct EbpfDataplane {
    /// Owns the loaded BPF maps and programs. Dropping this releases
    /// kernel-side resources. Field is kept live so the BPF object's
    /// maps/programs survive across `Dataplane` trait calls.
    #[allow(dead_code)]
    bpf: aya::Ebpf,

    /// Typed handle to the SERVICE_MAP outer HoM. Owns the FD shared
    /// with the kernel-side ELF declaration via `pinning = ByName`
    /// — the FD aya's loader recovered from the bpffs pin path is
    /// the same FD this handle's `OwnedFd` carries (kernel
    /// ref-counted; userspace and kernel see the same map identity).
    service_map: crate::maps::hash_of_maps::HashOfMapsHandle<
        crate::maps::ServiceKey,
        u32, // BackendId raw u32 — the inner ARRAY's value type
    >,

    /// Userspace handle to BACKEND_MAP (regular HASH; aya supports
    /// it natively). Owns the read/write surface for resolved
    /// backend records.
    ///
    /// Wrapped in `parking_lot::Mutex` because aya's
    /// `HashMap::insert` / `remove` take `&mut self`, but
    /// `Dataplane::update_service` is `&self` (the trait surface is
    /// the canonical interior-mutability boundary for BPF map
    /// updates). The lock is held only for the duration of the BPF
    /// syscalls — never across `.await`.
    backend_map: parking_lot::Mutex<
        aya::maps::HashMap<aya::maps::MapData, u32, crate::maps::BackendEntryPod>,
    >,

    /// Userspace handle to REVERSE_NAT_MAP (regular HASH; aya
    /// supports it natively). Keys = `BackendKeyPod { backend_ip,
    /// backend_port, proto, _pad }` host-order; values = `VipPod {
    /// vip_ip, vip_port, _pad }` host-order. Populated by
    /// `update_service` so the `xdp_reverse_nat_lookup` program
    /// (attached on the backend-facing veth ingress per ADR-0045)
    /// can rewrite source 5-tuple back to the VIP on response
    /// packets.
    ///
    /// Slice 05-04: promotion from the in-memory `BTreeMap`
    /// placeholder in `maps/reverse_nat_map_handle.rs` to the real
    /// BPF map. Same `parking_lot::Mutex` rationale as
    /// `backend_map` — interior mutability across the `&self` trait
    /// surface, lock dropped before any `.await`.
    reverse_nat_map: parking_lot::Mutex<
        aya::maps::HashMap<aya::maps::MapData, crate::maps::BackendKeyPod, crate::maps::VipPod>,
    >,

    /// Collision-free `BackendId` allocator per ADR-0046. Replaces
    /// the multiplicative-hash derivation with a monotonic counter +
    /// memo table. Lock held briefly during `update_service` —
    /// dropped before any `.await`.
    backend_id_alloc: parking_lot::Mutex<crate::allocators::BackendIdAllocator>,

    /// Per-service `BackendId` set tracker. Used by step 4 of the
    /// 5-step atomic swap (orphan GC) to compute the union of every
    /// active service's BackendIds — the "live set" against which
    /// BACKEND_MAP is swept. `BTreeMap` / `BTreeSet` per
    /// `.claude/rules/development.md` § "Ordered-collection choice"
    /// — both structures are iterated by the GC sweep (the union
    /// computation), and deterministic order is the right default
    /// even though the maps' point-access shape would technically
    /// permit `HashMap`.
    ///
    /// Lifecycle: `update_service` overwrites the entry for the
    /// active service-key with the new BackendId set BEFORE the GC
    /// sweep runs, so the GC sees the post-update live set.
    /// "Remove service" semantics (empty backend list) clear the
    /// entry.
    service_backends: parking_lot::Mutex<
        std::collections::BTreeMap<crate::maps::ServiceKey, std::collections::BTreeSet<u32>>,
    >,

    /// Per-service `BackendKeyPod` set tracker for REVERSE_NAT_MAP
    /// purge (Slice 05-04 / S-2.2-18). Records the
    /// `(backend_ip, backend_port, proto)` keys this service
    /// installed into REVERSE_NAT_MAP on the previous update; the
    /// diff against the new set drives the lockstep delete on
    /// backends that are no longer in the service.
    ///
    /// Without this tracker, removed backends would leave stale
    /// REVERSE_NAT_MAP entries (architecture.md § 11 + S-2.2-18
    /// purge invariant). Tracking per-service enables the
    /// cross-service union check: on purge, only entries absent
    /// from the union of ALL active services' key sets are
    /// deleted — preventing accidental cross-service deletion
    /// when two services share a backend address.
    service_reverse_nat_keys: parking_lot::Mutex<
        std::collections::BTreeMap<
            crate::maps::ServiceKey,
            std::collections::BTreeSet<crate::maps::BackendKeyPod>,
        >,
    >,

    /// Path to the bpffs directory holding the SERVICE_MAP pin.
    /// Production: `/sys/fs/bpf/overdrive`. Tests: per-test tempdir
    /// under `/sys/fs/bpf/overdrive-test-<rand>`. The pin file is
    /// `<pin_dir>/SERVICE_MAP`. Retained so [`Drop`] can clean up
    /// in the production constructor's failure paths; pins survive
    /// process exit otherwise (kernel reaps the underlying map once
    /// refcount=0).
    #[allow(dead_code)]
    pin_dir: std::path::PathBuf,

    /// Owns the forward-path XDP attachment (`xdp_service_map_lookup`
    /// on the client-facing iface ingress). Dropping detaches the
    /// program. Read only via Drop.
    #[allow(dead_code)]
    _xdp_forward_link: aya::programs::xdp::XdpLinkId,

    /// Owns the reverse-path XDP attachment
    /// (`xdp_reverse_nat_lookup` on the backend-facing iface
    /// ingress). Per ADR-0045 § Decision: reverse-NAT is performed in
    /// XDP at the backend-facing veth ingress (replacing the
    /// pre-pivot `tc_reverse_nat` egress attach). Dropping detaches
    /// the program.
    #[allow(dead_code)]
    _xdp_reverse_link: aya::programs::xdp::XdpLinkId,

    /// Userspace handle to LOCAL_BACKEND_MAP per ADR-0053 § 1.
    /// Populated by `register_local_backend` /
    /// `deregister_local_backend`; the
    /// `cgroup_connect4_service` kernel program reads it on every
    /// `connect(2)` from a process inside the attach cgroup.
    local_backend_map: crate::maps::local_backend_map_handle::LocalBackendMapHandle,

    /// Userspace handle to REVERSE_LOCAL_MAP per ADR-0053 rev
    /// 2026-06-05 (GH #200). Populated reverse-first by
    /// `register_local_backend` (DDD-1) — keyed on the backend
    /// identity `(backend_ip, backend_port, proto)` → vip. The
    /// `cgroup_recvmsg4_service` kernel program reads it on every
    /// unconnected `recvmsg(2)` to rewrite the reply source the app
    /// reads backend→VIP. DISTINCT from `reverse_nat_map` (the XDP
    /// connected/remote wire path).
    reverse_local_map: crate::maps::reverse_local_map_handle::ReverseLocalMapHandle,

    /// Owns the `cgroup_connect4_service` cgroup_sock_addr
    /// attachment. Dropping detaches the program per ADR-0053
    /// § "Consequences" — Reliability — recoverability.
    ///
    /// **Detach symmetry with XDP**: this field follows the same
    /// RAII shape as `_xdp_forward_link` / `_xdp_reverse_link`
    /// above — aya's `CgroupSockAddrLinkId` is a typed
    /// `bpf_link_create`-backed handle whose `Drop` impl invokes
    /// `close(2)` on the link fd, which detaches the program from
    /// the cgroup at the kernel-link refcount boundary (mirror of
    /// `aya::programs::xdp::XdpLinkId::drop` for XDP). The whole
    /// struct's drop order — fields drop in declaration order, so
    /// XDP links detach first, the BPF object's maps stay alive
    /// until the cgroup link drops, then the cgroup link releases
    /// last — is what makes the bpffs pin cleanup in the explicit
    /// `Drop for EbpfDataplane` impl below sequence-safe. No
    /// explicit detach call is needed; the same SIGKILL-skips-Drop
    /// caveat documented for the XDP path applies here, and the
    /// operator-side cleanup discipline in
    /// `.claude/rules/debugging.md` § "Leftover XDP attachments
    /// across runs" extends to cgroup_sock_addr attachments by the
    /// same mechanism (an unclean shutdown leaves the program
    /// attached until the next `EbpfDataplane::new` replaces it
    /// or an operator runs the equivalent `bpftool cgroup detach`
    /// sweep).
    #[allow(dead_code)]
    _cgroup_connect4_link: aya::programs::cgroup_sock_addr::CgroupSockAddrLinkId,

    /// Owns the `cgroup_sendmsg4_service` attachment (ADR-0053 rev
    /// 2026-06-05, GH #200). The per-datagram forward analogue of
    /// `connect4` — fires on every unconnected IPv4 `sendmsg`/`sendto`
    /// from a process inside the attach cgroup, rewriting the
    /// destination VIP→backend over `LOCAL_BACKEND_MAP`. Same RAII
    /// detach-on-drop shape as `_cgroup_connect4_link`.
    #[allow(dead_code)]
    _cgroup_sendmsg4_link: aya::programs::cgroup_sock_addr::CgroupSockAddrLinkId,

    /// Owns the `cgroup_recvmsg4_service` attachment (ADR-0053 rev
    /// 2026-06-05, GH #200). The reply-source rewrite — fires on every
    /// unconnected IPv4 `recvmsg`/`recvfrom`, rewriting the reply
    /// source backend→VIP over `REVERSE_LOCAL_MAP` so a same-host UDP
    /// client reads the VIP as the reply source. Same RAII
    /// detach-on-drop shape as `_cgroup_connect4_link`.
    #[allow(dead_code)]
    _cgroup_recvmsg4_link: aya::programs::cgroup_sock_addr::CgroupSockAddrLinkId,

    /// Userspace handle to `REVERSE_LOCAL_MISS_COUNTER` — the single-slot
    /// `BPF_MAP_TYPE_PERCPU_ARRAY<u64>` the `cgroup_recvmsg4_service`
    /// program bumps on every `REVERSE_LOCAL_MAP` miss (ADR-0053 § D3
    /// rev 2026-06-05b / UI-1). A miss is the common non-service-reply
    /// case (recvmsg4 fires on ALL subtree unconnected UDP); the counter
    /// is observable-but-inert — its incrementing has no effect on the
    /// source the app reads (the miss path is a pure no-op). Read-back
    /// via [`Self::reverse_local_miss_count`] sums per-CPU at read time
    /// (the `DROP_COUNTER` `aggregate_per_cpu` precedent).
    reverse_local_miss_counter: aya::maps::PerCpuArray<aya::maps::MapData, u64>,

    /// Attach path the `cgroup_connect4_service` program is bound
    /// to. Retained for the operator-surfaced
    /// `DataplaneError::LocalBackendProbe` error message context
    /// per ADR-0053 § 6.
    #[allow(dead_code)]
    cgroup_attach_path: std::path::PathBuf,

    /// Test-only failure-injection seam for [`Self::probe`]. When
    /// `Some(fault)`, `probe()` short-circuits to `Err(fault.clone())`
    /// BEFORE touching `BACKEND_MAP`. The seam is gated behind
    /// `#[cfg(any(test, feature = "integration-tests"))]`; production
    /// builds compile the field out entirely. Use by S-BDB-14 to
    /// drive the `DataplaneBootError::Probe` mapping arm without
    /// corrupting the BACKEND_MAP itself.
    #[cfg(any(test, feature = "integration-tests"))]
    probe_fault: parking_lot::Mutex<Option<DataplaneError>>,
}

impl EbpfDataplane {
    /// Construct an `EbpfDataplane` by:
    ///
    /// 1. Resolving `client_iface` and `backend_iface` names →
    ///    ifindexes (typed `IfaceNotFound` on `ENODEV`/`ENOENT`).
    /// 2. Pre-creating + pre-pinning the SERVICE_MAP outer HoM at
    ///    `<pin_dir>/SERVICE_MAP`. Required: aya 0.13.x's ELF loader
    ///    cannot create HoM (no `inner_map_fd` support in
    ///    `bpf_create_map`). The pin-by-name flow lets aya recover
    ///    the FD via `BPF_OBJ_GET` and reuse it.
    /// 3. Loading the BPF ELF via `EbpfLoader::map_pin_path(pin_dir)`
    ///    so aya's loader picks up the pinned outer FD.
    /// 4. Recovering the BACKEND_MAP and REVERSE_NAT_MAP typed
    ///    handles (regular HASH; aya supports them natively).
    /// 5. Attaching `xdp_service_map_lookup` to `client_iface` (the
    ///    forward-path ingress) with native-first → SKB fallback on
    ///    `EOPNOTSUPP`/`ENOTSUP`.
    /// 6. Attaching `xdp_reverse_nat_lookup` to `backend_iface` (the
    ///    reverse-path ingress) with the same fallback shape. Per
    ///    ADR-0045 § Decision: reverse-NAT is performed in XDP at
    ///    the backend-facing veth ingress (replacing the pre-pivot
    ///    `tc_reverse_nat` egress attach).
    ///
    /// `pin_dir` MUST be an existing bpffs mount (production passes
    /// `/sys/fs/bpf/overdrive` via [`Self::new`]; tests pass a per-
    /// test tempdir under `/sys/fs/bpf/overdrive-test-<rand>` via
    /// [`Self::new_with_pin_dir`]). The directory's parent must
    /// already exist; the directory itself is created if missing.
    #[allow(clippy::too_many_lines)]
    pub fn new_with_pin_dir(
        client_iface: &str,
        backend_iface: &str,
        pin_dir: &std::path::Path,
        cgroup_attach_path: &std::path::Path,
    ) -> Result<Self, DataplaneError> {
        use aya::programs::{CgroupAttachMode, CgroupSockAddr, Xdp, XdpFlags};
        use nix::errno::Errno;
        use nix::net::if_::if_nametoindex;

        use crate::maps::hash_of_maps::HashOfMapsHandle;
        use crate::maps::local_backend_map_handle::LocalBackendMapHandle;
        use crate::maps::reverse_local_map_handle::{
            ReverseLocalEntryPod, ReverseLocalKeyPod, ReverseLocalMapHandle,
        };
        use crate::maps::{
            BackendEntryPod, BackendKeyPod, LocalBackendEntry, LocalServiceKey, ServiceKey, VipPod,
        };

        // Resolve both iface names → ifindexes. ENODEV / ENOENT map
        // to the typed IfaceNotFound variant. Both ifaces are
        // resolved up-front so a missing backend-facing iface does
        // not surface only after the forward-path attach has
        // partially succeeded.
        if_nametoindex(client_iface).map_err(|errno| match errno {
            Errno::ENODEV | Errno::ENOENT => {
                DataplaneError::IfaceNotFound { iface: client_iface.to_string() }
            }
            other => DataplaneError::LoadFailed(format!("if_nametoindex({client_iface}): {other}")),
        })?;
        if_nametoindex(backend_iface).map_err(|errno| match errno {
            Errno::ENODEV | Errno::ENOENT => {
                DataplaneError::IfaceNotFound { iface: backend_iface.to_string() }
            }
            other => {
                DataplaneError::LoadFailed(format!("if_nametoindex({backend_iface}): {other}"))
            }
        })?;

        // Ensure the pin directory exists. Failure here is a
        // configuration error (parent isn't a bpffs mount, or
        // CAP_SYS_ADMIN missing); surface as LoadFailed with the
        // originating errno text.
        std::fs::create_dir_all(pin_dir).map_err(|e| {
            DataplaneError::LoadFailed(format!("create pin directory {}: {e}", pin_dir.display()))
        })?;

        // Clean any stale SERVICE_MAP pin from a prior unclean
        // shutdown. `unlink` against a non-existent path is fine; we
        // only error if the path exists AND we cannot unlink it.
        let pin_path = pin_dir.join(SERVICE_MAP_NAME);
        if let Err(e) = std::fs::remove_file(&pin_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(DataplaneError::LoadFailed(format!(
                    "unlink stale pin {}: {e}",
                    pin_path.display()
                )));
            }
        }

        // Pre-create + pre-pin SERVICE_MAP. Outer max_entries =
        // 4096 (architecture.md § 10); inner ARRAY size = 256
        // (Q5=A). Failure here surfaces as MapAllocFailed (the typed
        // S-2.2-11 variant via the From impl on HashOfMapsError).
        let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
            SERVICE_MAP_NAME,
            SERVICE_MAP_OUTER_CAPACITY,
            SERVICE_MAP_INNER_CAPACITY,
            pin_dir,
        )
        .map_err(|e| match e {
            crate::maps::hash_of_maps::HashOfMapsError::MapAllocFailed { source } => {
                DataplaneError::MapAllocFailed { source }
            }
            crate::maps::hash_of_maps::HashOfMapsError::Syscall(source) => {
                DataplaneError::LoadFailed(format!("SERVICE_MAP pin: {source}"))
            }
        })?;

        // Load the BPF ELF with the pin path set so aya's loader
        // reuses the pre-pinned outer FD via BPF_OBJ_GET.
        // `allow_unsupported_maps()` is mandatory: aya 0.13.x's
        // `Map` enum has no `HashOfMaps` variant, so SERVICE_MAP
        // surfaces as `Map::Unsupported(MapData)`. The loader
        // rejects unsupported maps by default; we accept the
        // variant because the userspace path doesn't go through
        // `bpf.take_map("SERVICE_MAP")` for HoM — we own the
        // typed `HashOfMapsHandle` separately. See research § A.2.
        //
        // The slice path of aya 0.13 (`EbpfLoader::load(&[u8])`)
        // rejects BTF-less ELFs in some configurations; `load_file`
        // is more tolerant. We materialise the embedded slice to a
        // temp file under `/tmp` (NOT under `pin_dir`, which is a
        // bpffs mount that rejects regular file writes) and load
        // from there. The file is removed on success — its bytes
        // are mmap'd by the kernel via the BPF program file
        // descriptors, which are kept alive by the `aya::Ebpf`
        // value.
        let bpf_temp_path =
            std::env::temp_dir().join(format!("overdrive_bpf-{}.o", std::process::id()));
        std::fs::write(&bpf_temp_path, OVERDRIVE_BPF_OBJ).map_err(|e| {
            DataplaneError::LoadFailed(format!(
                "write embedded BPF object to {}: {e}",
                bpf_temp_path.display()
            ))
        })?;
        let bpf = aya::EbpfLoader::new()
            .map_pin_path(pin_dir)
            .allow_unsupported_maps()
            .load_file(&bpf_temp_path)
            .map_err(|e| DataplaneError::LoadFailed(format!("aya load: {e}")));
        // Best-effort cleanup of the temp file — even on load
        // failure we want to remove it so /tmp does not
        // accumulate stale bytes.
        let _ = std::fs::remove_file(&bpf_temp_path);
        let mut bpf = bpf?;

        // Recover BACKEND_MAP typed handle.
        let backend_map = aya::maps::HashMap::<_, u32, BackendEntryPod>::try_from(
            bpf.take_map(BACKEND_MAP_NAME).ok_or_else(|| {
                DataplaneError::LoadFailed("BACKEND_MAP not found in BPF object".into())
            })?,
        )
        .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP try_from: {e}")))?;

        // Recover REVERSE_NAT_MAP typed handle. Slice 05-04
        // promotion: this is the production map the
        // `xdp_reverse_nat_lookup` program reads on every
        // backend-response packet (per ADR-0045 — XDP at the
        // backend-facing veth ingress, replacing the pre-pivot
        // TC-egress attach). Userspace populates entries in
        // `update_service`; missing entries cause `XDP_PASS` and
        // the kernel routes the unrewritten packet through the
        // normal stack (the architectural intent — late responses
        // from removed backends are non-LB traffic, see S-2.2-18).
        let reverse_nat_map = aya::maps::HashMap::<_, BackendKeyPod, VipPod>::try_from(
            bpf.take_map(REVERSE_NAT_MAP_NAME).ok_or_else(|| {
                DataplaneError::LoadFailed("REVERSE_NAT_MAP not found in BPF object".into())
            })?,
        )
        .map_err(|e| DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP try_from: {e}")))?;

        // Load + attach the service-map lookup program.
        let prog: &mut Xdp = bpf
            .program_mut("xdp_service_map_lookup")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "xdp_service_map_lookup program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| DataplaneError::LoadFailed(format!("xdp program type: {e}")))?;
        prog.load()
            .map_err(|e| DataplaneError::LoadFailed(format!("xdp_service_map_lookup.load: {e}")))?;

        // Native-first attach with documented EOPNOTSUPP/ENOTSUP →
        // SKB fallback. Same shape as the prior xdp_pass attach
        // (S-2.2-02). Forward path attaches on the client-facing
        // iface ingress. Classification policy lives in
        // `classify_attach_result` — see its docstring for the
        // mutation-testing rationale (Lima virtio-net never
        // exercises the Fallback arm; unit tests against synthetic
        // `SyscallError` values do).
        let xdp_forward_link = match classify_attach_result(
            prog.attach(client_iface, XdpFlags::DRV_MODE),
        ) {
            AttachOutcome::Native(link) => link,
            AttachOutcome::Fallback { syscall } => {
                tracing::warn!(
                    name: "xdp.attach.fallback_generic",
                    iface = %client_iface,
                    syscall = %syscall,
                    "native XDP attach not supported by driver; falling back to generic (SKB) mode"
                );
                prog.attach(client_iface, XdpFlags::SKB_MODE).map_err(|e| {
                    DataplaneError::LoadFailed(format!(
                        "xdp_service_map_lookup.attach({client_iface}, SKB_MODE) after native fallback: {e}"
                    ))
                })?
            }
            AttachOutcome::Propagate(e) => {
                // Classify EBUSY into the honest slot-collision
                // diagnostic (ADR-0061 § 5 / D3) before falling
                // through to the masking DRV_MODE string.
                if let Some(busy) = classify_iface_xdp_slot_busy(&e, client_iface) {
                    return Err(busy);
                }
                return Err(DataplaneError::LoadFailed(format!(
                    "xdp_service_map_lookup.attach({client_iface}, DRV_MODE): {e}"
                )));
            }
        };

        // Attach `xdp_reverse_nat_lookup` to `backend_iface` ingress.
        //
        // Per ADR-0045 § Decision: reverse-NAT moves from
        // TC-egress (pre-pivot) to XDP at the backend-facing veth
        // ingress. Same native-first → SKB fallback shape as the
        // forward-path attach above; the fallback contract pinned
        // by Slice 01 unit tests in `lib.rs` covers both call sites.
        let reverse_prog: &mut Xdp = bpf
            .program_mut("xdp_reverse_nat_lookup")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "xdp_reverse_nat_lookup program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("xdp_reverse_nat_lookup program type: {e}"))
            })?;
        reverse_prog
            .load()
            .map_err(|e| DataplaneError::LoadFailed(format!("xdp_reverse_nat_lookup.load: {e}")))?;
        let xdp_reverse_link = match classify_attach_result(
            reverse_prog.attach(backend_iface, XdpFlags::DRV_MODE),
        ) {
            AttachOutcome::Native(link) => link,
            AttachOutcome::Fallback { syscall } => {
                tracing::warn!(
                    name: "xdp.attach.fallback_generic",
                    iface = %backend_iface,
                    syscall = %syscall,
                    "native XDP attach not supported by driver; falling back to generic (SKB) mode"
                );
                reverse_prog.attach(backend_iface, XdpFlags::SKB_MODE).map_err(|e| {
                    DataplaneError::LoadFailed(format!(
                        "xdp_reverse_nat_lookup.attach({backend_iface}, SKB_MODE) after native fallback: {e}"
                    ))
                })?
            }
            AttachOutcome::Propagate(e) => {
                // Classify EBUSY into the honest slot-collision
                // diagnostic (ADR-0061 § 5 / D3) before falling
                // through to the masking DRV_MODE string.
                if let Some(busy) = classify_iface_xdp_slot_busy(&e, backend_iface) {
                    return Err(busy);
                }
                return Err(DataplaneError::LoadFailed(format!(
                    "xdp_reverse_nat_lookup.attach({backend_iface}, DRV_MODE): {e}"
                )));
            }
        };

        // ADR-0053 § 1 — recover LOCAL_BACKEND_MAP typed handle.
        // Regular HASH; aya supports it natively. The kernel
        // `#[map]` declaration in
        // `crates/overdrive-bpf/src/maps/local_backend_map.rs` is
        // what makes this map present in the loaded ELF.
        let local_backend_map_inner =
            aya::maps::HashMap::<_, LocalServiceKey, LocalBackendEntry>::try_from(
                bpf.take_map(LOCAL_BACKEND_MAP_NAME).ok_or_else(|| {
                    DataplaneError::LoadFailed("LOCAL_BACKEND_MAP not found in BPF object".into())
                })?,
            )
            .map_err(|e| DataplaneError::LoadFailed(format!("LOCAL_BACKEND_MAP try_from: {e}")))?;
        let local_backend_map = LocalBackendMapHandle::new(local_backend_map_inner);

        // ADR-0053 rev 2026-06-05 (GH #200) — recover REVERSE_LOCAL_MAP
        // typed handle. Regular HASH; aya supports it natively. The
        // kernel `#[map]` declaration in
        // `crates/overdrive-bpf/src/maps/reverse_local_map.rs` makes
        // this map present in the loaded ELF. DISTINCT from
        // REVERSE_NAT_MAP (the XDP wire path) — keyed on the backend
        // identity, value = the original VIP (address AND port).
        let reverse_local_map_inner =
            aya::maps::HashMap::<_, ReverseLocalKeyPod, ReverseLocalEntryPod>::try_from(
                bpf.take_map(REVERSE_LOCAL_MAP_NAME).ok_or_else(|| {
                    DataplaneError::LoadFailed("REVERSE_LOCAL_MAP not found in BPF object".into())
                })?,
            )
            .map_err(|e| DataplaneError::LoadFailed(format!("REVERSE_LOCAL_MAP try_from: {e}")))?;
        let reverse_local_map = ReverseLocalMapHandle::new(reverse_local_map_inner);

        // ADR-0053 § D3 (rev 2026-06-05b / UI-1) — recover the
        // REVERSE_LOCAL_MISS_COUNTER PerCpuArray<u64>. The kernel
        // `#[map]` declaration in
        // `crates/overdrive-bpf/src/maps/reverse_local_miss_counter.rs`
        // makes this map present in the loaded ELF. Single slot
        // (`SLOT_REVERSE_MISS = 0`); userspace sums per-CPU at read time
        // (the DROP_COUNTER `aggregate_per_cpu` precedent).
        let reverse_local_miss_counter = aya::maps::PerCpuArray::<_, u64>::try_from(
            bpf.take_map("REVERSE_LOCAL_MISS_COUNTER").ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "REVERSE_LOCAL_MISS_COUNTER not found in BPF object".into(),
                )
            })?,
        )
        .map_err(|e| {
            DataplaneError::LoadFailed(format!("REVERSE_LOCAL_MISS_COUNTER try_from: {e}"))
        })?;

        // ADR-0053 § 1 — load + attach cgroup_connect4_service.
        // Open the cgroup directory FD (read-only is sufficient;
        // aya's `attach` passes it through to
        // `bpf_link_create(LinkTarget::Fd(cgroup_fd))`). The
        // operator-supplied cgroup_attach_path MUST exist and be
        // an ancestor of every workload cgroup per ADR-0053 § 7.
        let cgroup_file = std::fs::File::open(cgroup_attach_path).map_err(|e| {
            DataplaneError::LoadFailed(format!(
                "open cgroup_attach_path {}: {e}",
                cgroup_attach_path.display()
            ))
        })?;

        let cgroup_prog: &mut CgroupSockAddr = bpf
            .program_mut("cgroup_connect4_service")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "cgroup_connect4_service program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("cgroup_connect4_service program type: {e}"))
            })?;
        // aya recovers the attach type from the kernel-side
        // `link_section = "cgroup/connect4"` emitted by the
        // `#[cgroup_sock_addr(connect4)]` macro; no additional
        // pinning here.
        cgroup_prog.load().map_err(|e| {
            DataplaneError::LoadFailed(format!("cgroup_connect4_service.load: {e}"))
        })?;
        let cgroup_link_id =
            cgroup_prog.attach(&cgroup_file, CgroupAttachMode::Single).map_err(|e| {
                DataplaneError::LoadFailed(format!(
                    "cgroup_connect4_service.attach({}): {e}",
                    cgroup_attach_path.display()
                ))
            })?;

        // ADR-0053 rev 2026-06-05 (GH #200) — load + attach the two new
        // unconnected-UDP cgroup hooks to the SAME cgroup ancestor
        // (DDD-5b). Each is a `CgroupSockAddr` program; aya recovers the
        // attach type from the kernel-side `link_section` the
        // `#[cgroup_sock_addr(sendmsg4)]` / `(recvmsg4)` macros emit. The
        // attaches happen AFTER connect4 so a partial failure leaves a
        // consistent "earlier hooks attached, later not" state the boot
        // error surfaces — not a silently half-wired dataplane.
        let sendmsg4_prog: &mut CgroupSockAddr = bpf
            .program_mut("cgroup_sendmsg4_service")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "cgroup_sendmsg4_service program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("cgroup_sendmsg4_service program type: {e}"))
            })?;
        sendmsg4_prog.load().map_err(|e| cgroup_load_error("cgroup_sendmsg4_service", &e))?;
        // ADR-0053 D5b/c — the attach() syscall IS the below-floor
        // preflight: a host below the sendmsg4 floor (< 4.18) rejects this
        // attach (EOPNOTSUPP/ENOSYS). Route through the typed
        // `CgroupSendRecvAttach` variant (never `LoadFailed(String)`) so
        // the composition root surfaces the refusal as
        // `health.startup.refused`. NO /proc/uname parse.
        let cgroup_sendmsg4_link_id = sendmsg4_prog
            .attach(&cgroup_file, CgroupAttachMode::Single)
            .map_err(|e| cgroup_attach_error("cgroup_sendmsg4_service", &e))?;

        let recvmsg4_prog: &mut CgroupSockAddr = bpf
            .program_mut("cgroup_recvmsg4_service")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "cgroup_recvmsg4_service program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("cgroup_recvmsg4_service program type: {e}"))
            })?;
        recvmsg4_prog.load().map_err(|e| cgroup_load_error("cgroup_recvmsg4_service", &e))?;
        // ADR-0053 D5b/c — the attach() syscall IS the below-floor
        // preflight: a host below the recvmsg4 floor (< 4.20) rejects this
        // attach (EOPNOTSUPP/ENOSYS). Route through the typed
        // `CgroupSendRecvAttach` variant (never `LoadFailed(String)`).
        let cgroup_recvmsg4_link_id = recvmsg4_prog
            .attach(&cgroup_file, CgroupAttachMode::Single)
            .map_err(|e| cgroup_attach_error("cgroup_recvmsg4_service", &e))?;

        Ok(Self {
            bpf,
            service_map,
            backend_map: parking_lot::Mutex::new(backend_map),
            reverse_nat_map: parking_lot::Mutex::new(reverse_nat_map),
            backend_id_alloc: parking_lot::Mutex::new(crate::allocators::BackendIdAllocator::new()),
            service_backends: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            service_reverse_nat_keys: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            pin_dir: pin_dir.to_path_buf(),
            _xdp_forward_link: xdp_forward_link,
            _xdp_reverse_link: xdp_reverse_link,
            local_backend_map,
            reverse_local_map,
            _cgroup_connect4_link: cgroup_link_id,
            _cgroup_sendmsg4_link: cgroup_sendmsg4_link_id,
            _cgroup_recvmsg4_link: cgroup_recvmsg4_link_id,
            reverse_local_miss_counter,
            cgroup_attach_path: cgroup_attach_path.to_path_buf(),
            #[cfg(any(test, feature = "integration-tests"))]
            probe_fault: parking_lot::Mutex::new(None),
        })
    }

    /// Construct an `EbpfDataplane` against the production pin
    /// directory (`/sys/fs/bpf/overdrive`). Tests use
    /// [`Self::new_with_pin_dir`] with a per-test tempdir.
    ///
    /// Per ADR-0045 § Operational the loader takes two ifaces:
    /// `client_iface` (forward-path; `xdp_service_map_lookup`
    /// ingress) and `backend_iface` (reverse-path;
    /// `xdp_reverse_nat_lookup` ingress).
    pub fn new(client_iface: &str, backend_iface: &str) -> Result<Self, DataplaneError> {
        Self::new_with_pin_dir(
            client_iface,
            backend_iface,
            std::path::Path::new(DEFAULT_PIN_DIR),
            std::path::Path::new(DEFAULT_CGROUP_ATTACH_PATH),
        )
    }

    /// Number of entries currently in REVERSE_NAT_MAP.
    ///
    /// Observability surface — used by Tier 3 integration tests
    /// (S-2.2-18 purge invariant verification). Iterates the BPF
    /// map's `keys()` generator and counts; returns the count plus
    /// any iteration error from the kernel.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// a `keys()` iteration step.
    pub fn reverse_nat_map_size(&self) -> Result<usize, DataplaneError> {
        let map = self.reverse_nat_map.lock();
        map.keys()
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.len())
            .map_err(|e| DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP keys(): {e}")))
    }

    /// Returns `true` if REVERSE_NAT_MAP contains an entry for the
    /// given `(backend_ip, backend_port)` keyed under TCP.
    ///
    /// Observability surface — companion to [`Self::reverse_nat_map_size`].
    /// Fixes `proto = TCP`; the per-proto variant is
    /// [`Self::reverse_nat_map_has_backend_proto`]. Since ADR-0060
    /// (GH #163) `EbpfDataplane::update_service` populates the
    /// REVERSE_NAT key with the `ServiceFrontend`'s proto byte, so a UDP
    /// service installs a proto=17 key this TCP-keyed helper will MISS —
    /// pass `Proto::Udp` to `reverse_nat_map_has_backend_proto`, or use
    /// `reverse_nat_map_size` for a proto-agnostic count.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// the lookup with anything other than `KeyNotFound` (which is
    /// the `Ok(false)` path).
    pub fn reverse_nat_map_has_backend(
        &self,
        ip: Ipv4Addr,
        port: u16,
    ) -> Result<bool, DataplaneError> {
        use overdrive_core::dataplane::backend_key::Proto;
        self.reverse_nat_map_has_backend_proto(ip, port, Proto::Tcp)
    }

    /// Returns `true` if REVERSE_NAT_MAP contains an entry for the
    /// given `(backend_ip, backend_port, proto)` 3-tuple key.
    ///
    /// Observability surface — the proto-aware companion to
    /// [`Self::reverse_nat_map_has_backend`] (which fixes `proto = TCP`).
    /// Per ADR-0060 D7 the REVERSE_NAT key carries the per-service L4
    /// protocol byte (6 = tcp, 17 = udp); a UDP service installs a
    /// proto=17 key that a TCP-keyed lookup would MISS. Tier 3 e2e tests
    /// that exercise the UDP reverse-NAT path (US-04) use this accessor
    /// to assert the `(backend_ip, 5353, udp)` key is present — the
    /// observable side-effect of the source rewrite the kernel
    /// `xdp_reverse_nat_lookup` performs on proto=17 responses.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// the lookup with anything other than `KeyNotFound` (the
    /// `Ok(false)` path).
    pub fn reverse_nat_map_has_backend_proto(
        &self,
        ip: Ipv4Addr,
        port: u16,
        proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<bool, DataplaneError> {
        use crate::maps::BackendKeyPod;

        let key = BackendKeyPod {
            ip_host: u32::from(ip),
            port_host: port,
            proto: proto.as_u8(),
            _pad: 0,
        };
        let map = self.reverse_nat_map.lock();
        match map.get(&key, 0) {
            Ok(_) => Ok(true),
            Err(aya::maps::MapError::KeyNotFound) => Ok(false),
            Err(e) => Err(DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP get: {e}"))),
        }
    }

    /// Snapshot of the keys (BackendIds) currently present in
    /// `BACKEND_MAP`. Returned in arbitrary order — callers that
    /// depend on stability should collect into a `BTreeSet`.
    ///
    /// Observability surface — used by Tier 3 integration tests
    /// (S-2.2-10 orphan-GC verification) and intended for production
    /// debug tooling. Does not violate the trait surface boundary
    /// because the `Dataplane` trait does not need this — it is an
    /// auxiliary read-side accessor on the concrete type, parallel
    /// to `drain_flow_events` (which IS on the trait because every
    /// implementation must surface telemetry).
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// a `keys()` iteration step (mid-iteration map mutation, kernel
    /// out-of-memory, etc).
    pub fn backend_map_keys(&self) -> Result<Vec<u32>, DataplaneError> {
        let backend_map = self.backend_map.lock();
        backend_map
            .keys()
            .collect::<Result<Vec<u32>, _>>()
            .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP keys(): {e}")))
    }

    /// Number of entries in the `BackendIdAllocator`'s memo table.
    ///
    /// Diagnostic surface — used by integration tests to verify that
    /// `release()` is called after orphan-GC sweeps on both the
    /// non-empty and empty-backend paths.
    #[must_use]
    pub fn allocator_memo_len(&self) -> usize {
        self.backend_id_alloc.lock().memo_len()
    }

    /// Returns `true` if SERVICE_MAP contains an outer slot for the
    /// given `(vip, port, proto)` key.
    ///
    /// Observability surface — used by Tier 3 integration tests to
    /// verify the empty-backend cleanup path removes the outer HoM
    /// slot. Step 02-01 widened the key with `proto`, so the diagnostic
    /// queries the exact `(vip, port, proto)` slot rather than a
    /// proto-agnostic one.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// the lookup with an error other than `ENOENT`.
    pub fn service_map_contains(
        &self,
        vip: std::net::Ipv4Addr,
        port: u16,
        proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<bool, DataplaneError> {
        use crate::maps::wire::ServiceKey;

        let key =
            ServiceKey { vip_host: u32::from(vip), port_host: port, proto: proto.as_u8(), _pad: 0 };
        let key_bytes = unsafe {
            core::slice::from_raw_parts(
                (&raw const key).cast::<u8>(),
                core::mem::size_of::<ServiceKey>(),
            )
        };
        match crate::sys::bpf::bpf_map_lookup_elem(
            self.service_map.as_fd(),
            key_bytes,
            core::mem::size_of::<u32>(),
        ) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(DataplaneError::LoadFailed(format!("SERVICE_MAP lookup: {e}"))),
        }
    }

    /// Snapshot of the `(BackendId, BackendEntryPod)` pairs currently
    /// present in `BACKEND_MAP`.
    ///
    /// Test-only — gated behind `cfg(any(test, feature = "integration-
    /// tests"))` per `backend-discovery-bridge-service-reachability/
    /// design/architecture.md` § 6.2 / Atlas Q1: the production crate's
    /// public surface MUST NOT widen the test-only inspector to
    /// non-test consumers. The narrower `backend_map_keys()` /
    /// `service_map_contains()` accessors above are kept public
    /// because they are consumed by production debug tooling (and by
    /// the in-feature S-2.2-10 orphan-GC verification); only the
    /// full-entry iterator is gated.
    ///
    /// Used by the walking-skeleton (S-BDB-01) to assert a backend
    /// matching `(host_ipv4, listener_port)` was written by the
    /// `update_service` path. Returned in arbitrary order — callers
    /// that need stability sort by the `(ipv4_host, port_host)` tuple.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects a
    /// `keys()` iteration step or a `get()` lookup for a key the prior
    /// iteration surfaced. The races that could surface here
    /// (mid-iteration map mutation, kernel out-of-memory) are the
    /// same that `backend_map_keys()` documents — the inspector
    /// simply chains a `get()` per key.
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn backend_map_entries(
        &self,
    ) -> Result<Vec<(u32, crate::maps::BackendEntryPod)>, DataplaneError> {
        let backend_map = self.backend_map.lock();
        let keys = backend_map
            .keys()
            .collect::<Result<Vec<u32>, _>>()
            .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP keys(): {e}")))?;
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let entry = backend_map
                .get(&key, 0)
                .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP get({key}): {e}")))?;
            entries.push((key, entry));
        }
        drop(backend_map);
        Ok(entries)
    }

    /// Earned-Trust probe per `backend-discovery-bridge-service-
    /// reachability` architecture.md § 5.4 and CLAUDE.md principle 12.
    ///
    /// Writes a sentinel `BACKEND_MAP` entry under `BackendId::PROBE
    /// = u32::MAX`, reads it back, asserts byte-equal, then deletes
    /// it. Proves the HoM pin-by-name reuse path + `BACKEND_MAP`
    /// programmability work on this kernel BEFORE accepting traffic.
    ///
    /// **Composition-root invariant**: the production boot path
    /// (`overdrive-control-plane::serve_with_config`) MUST call
    /// `probe().await` AFTER [`Self::new`] / [`Self::new_with_pin_dir`]
    /// succeeds and BEFORE the first downstream dataplane operation.
    /// On failure the boot path refuses to start with a structured
    /// `health.startup.refused` event (`reason = "dataplane.probe"`)
    /// and surfaces `DataplaneBootError::Probe { source }` to the
    /// operator. The error chain's `Display` contains either
    /// "probe: round-trip mismatch" (the sentinel byte-equality
    /// check) or "probe: BACKEND_MAP <step>: <underlying-error>"
    /// (one of the three map syscalls).
    ///
    /// # Preconditions
    ///
    /// None. The probe operates on the typed `BACKEND_MAP` handle the
    /// constructor owns; no external resource is required beyond what
    /// the constructor already guarantees (loaded BPF ELF, attached
    /// XDP, pinned SERVICE_MAP).
    ///
    /// # Postconditions
    ///
    /// On `Ok(())`: `BACKEND_MAP` is byte-equal to its pre-probe state
    /// — the sentinel was written, read back, and deleted. A caller
    /// that immediately inspects `BACKEND_MAP::get(u32::MAX, 0)` MUST
    /// observe `None`.
    ///
    /// On `Err(DataplaneError::LoadFailed(msg))`: `BACKEND_MAP` MAY
    /// contain partial sentinel state (if the failure occurred between
    /// the insert and the delete). The caller MUST NOT use the
    /// dataplane after a failed probe — the Earned-Trust contract is
    /// violated and the kernel's view of the map is undefined relative
    /// to userspace expectations.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] with a message starting
    /// with `"probe: BACKEND_MAP insert: ..."`, `"probe: BACKEND_MAP
    /// get: ..."`, `"probe: round-trip mismatch ..."`, or `"probe:
    /// BACKEND_MAP delete: ..."` depending on which step rejected.
    /// Returns [`DataplaneError::LocalBackendProbe`] when the
    /// `LOCAL_BACKEND_MAP` sentinel round-trip fails, and
    /// [`DataplaneError::ReverseLocalProbe`] when the `REVERSE_LOCAL_MAP`
    /// sentinel round-trip fails (ADR-0053 rev D5b/c reverse-path
    /// Earned-Trust extension) — both surfaced by the composition root as
    /// `health.startup.refused`.
    // `#[allow(clippy::unused_async)]` is required because the probe
    // body uses only synchronous parking_lot::Mutex guards against
    // the `BackendMapHandle`; there is no `.await` inside. The
    // function MUST stay async because the boot path calls
    // `.probe().await` (composition root invariant "wire then probe
    // then use" per CLAUDE.md principle 12) and future probe
    // additions (TLS-handshake roundtrip, kernel-side ringbuf drain)
    // will need real awaits. Treating the signature as async-stable
    // matches the rest of the `Dataplane` trait surface.
    #[allow(clippy::unused_async)]
    pub async fn probe(&self) -> Result<(), DataplaneError> {
        use crate::maps::wire::BackendEntryPod;

        // Sentinel BackendId — `u32::MAX` is reserved for the probe
        // per architecture.md § 5.4. Real BackendIds come from the
        // monotonic-counter allocator and never reach this value.
        const SENTINEL_BACKEND_ID: u32 = u32::MAX;

        // Test-only short-circuit: when the failure-injection seam is
        // armed, surface the configured fault BEFORE touching the
        // kernel. The seam is the single production-shape concession
        // permitted by `.claude/rules/development.md` § "Production
        // code is not shaped by simulation" — it is gated behind
        // `cfg(any(test, feature = "integration-tests"))`, the field
        // is compiled out in production builds, and the branch
        // collapses to nothing for the production code path. The
        // `.take()` runs against a freshly-taken guard that drops
        // before the `if let` so clippy's
        // `significant_drop_in_scrutinee` lint is satisfied.
        #[cfg(any(test, feature = "integration-tests"))]
        {
            let armed_fault = self.probe_fault.lock().take();
            if let Some(fault) = armed_fault {
                return Err(fault);
            }
        }

        // Sentinel POD — `127.0.0.1` host-order (`0x7F_00_00_01`),
        // port = 0, weight = 0, healthy = 0, `_pad` = zeroed.
        // Field names match the actual `BackendEntryPod` struct
        // (`ipv4_host` / `port_host`), not the architecture.md
        // pseudo-code spelling (`ipv4` / `port`) which predates the
        // 05-04 wire-type rename.
        let sentinel = BackendEntryPod {
            ipv4_host: 0x7F_00_00_01,
            port_host: 0,
            weight: 0,
            healthy: 0,
            _pad: [0; 3],
        };

        // Step 1 — write.
        self.backend_map
            .lock()
            .insert(SENTINEL_BACKEND_ID, sentinel, 0)
            .map_err(|e| DataplaneError::LoadFailed(format!("probe: BACKEND_MAP insert: {e}")))?;

        // Step 2 — read-back. Bind the lookup `Result` to a local so
        // the `parking_lot::MutexGuard` drops before the `match`
        // arm runs — keeps clippy's
        // `significant_drop_in_scrutinee` lint satisfied without
        // holding the lock across the error-formatting path.
        let read_back = self.backend_map.lock().get(&SENTINEL_BACKEND_ID, 0);
        let got: Option<BackendEntryPod> = match read_back {
            Ok(v) => Some(v),
            Err(aya::maps::MapError::KeyNotFound) => None,
            Err(e) => {
                return Err(DataplaneError::LoadFailed(format!("probe: BACKEND_MAP get: {e}")));
            }
        };

        // Step 3 — assert byte-equal. Mismatch (either `None` or a
        // different value) is the structural signal that
        // `BACKEND_MAP` programmability is broken on this kernel.
        if got != Some(sentinel) {
            // Best-effort cleanup before bailing — leave `BACKEND_MAP`
            // in a clean state if we can. Errors here are swallowed:
            // the round-trip mismatch is the real story and a delete
            // failure on an already-broken map would only mask it.
            let _ = self.backend_map.lock().remove(&SENTINEL_BACKEND_ID);
            return Err(DataplaneError::LoadFailed(format!(
                "probe: round-trip mismatch (got {got:?}, want {sentinel:?})"
            )));
        }

        // Step 4 — delete. After a clean delete, `BACKEND_MAP` is
        // byte-equal to its pre-probe state.
        self.backend_map
            .lock()
            .remove(&SENTINEL_BACKEND_ID)
            .map_err(|e| DataplaneError::LoadFailed(format!("probe: BACKEND_MAP delete: {e}")))?;

        // ADR-0053 § 6 — Earned-Trust probe extension. Sentinel
        // round-trip against LOCAL_BACKEND_MAP confirms the cgroup
        // hook attached AND the map is programmable end-to-end.
        // Sentinel: (vip=0.0.0.0, vip_port=0) → (backend=0.0.0.0:0).
        // The sentinel is reserved per the typed-handle convention;
        // production allocator-issued VIPs never use 0.0.0.0.
        let sentinel_vip = Ipv4Addr::UNSPECIFIED;
        let sentinel_vip_port: u16 = 0;
        let sentinel_backend = std::net::SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        // Sentinel proto is arbitrary — the round-trip only confirms the
        // map is programmable end-to-end. TCP is the conventional choice.
        let sentinel_proto = overdrive_core::dataplane::backend_key::Proto::Tcp;

        self.local_backend_map
            .upsert(sentinel_vip, sentinel_vip_port, sentinel_backend, sentinel_proto)
            .map_err(|e| DataplaneError::LocalBackendProbe {
                message: format!("LOCAL_BACKEND_MAP sentinel insert: {e}"),
            })?;

        let got = self
            .local_backend_map
            .get(sentinel_vip, sentinel_vip_port, sentinel_proto)
            .map_err(|e| DataplaneError::LocalBackendProbe {
                message: format!("LOCAL_BACKEND_MAP sentinel get: {e}"),
            })?;
        if got.is_none() {
            return Err(DataplaneError::LocalBackendProbe {
                message: "LOCAL_BACKEND_MAP sentinel round-trip missed: got None".to_string(),
            });
        }

        self.local_backend_map.remove(sentinel_vip, sentinel_vip_port, sentinel_proto).map_err(
            |e| DataplaneError::LocalBackendProbe {
                message: format!("LOCAL_BACKEND_MAP sentinel delete: {e}"),
            },
        )?;

        // ADR-0053 rev D5b/c (GH #200) — Earned-Trust reverse-path
        // extension. The two unconnected-UDP hooks are already attached
        // (in `EbpfDataplane::new`, routed through the typed
        // `CgroupSendRecvAttach` variant on a below-floor kernel). This
        // sentinel round-trip confirms the reverse backend→VIP lookup
        // path is programmable end-to-end BEFORE the dataplane is declared
        // usable ("wire → probe → use").
        self.probe_reverse_local()?;

        Ok(())
    }

    /// REVERSE_LOCAL_MAP sentinel round-trip — the reverse-path half of
    /// the Earned-Trust probe (ADR-0053 rev D5b/c, GH #200). Writes,
    /// reads-back, and deletes a reserved `(0.0.0.0:0, Udp) → 0.0.0.0`
    /// sentinel; a miss is the structural signal the reverse lookup path
    /// is not programmable on this kernel. The 0.0.0.0 backend identity is
    /// reserved — production allocator-issued backends never use it.
    ///
    /// Extracted from [`Self::probe`] to keep that method within the
    /// per-fn line budget; it is the sole caller.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::ReverseLocalProbe`] when the sentinel
    /// insert, read-back, round-trip assertion, or delete fails.
    fn probe_reverse_local(&self) -> Result<(), DataplaneError> {
        use crate::maps::reverse_local_map_handle::{ReverseLocalKeyPod, ReverseLocalMapValue};
        use overdrive_core::dataplane::backend_key::{BackendKey, Proto};

        let sentinel_backend_ip = Ipv4Addr::UNSPECIFIED;
        let sentinel_backend_port: u16 = 0;
        let sentinel_proto = Proto::Udp;
        let sentinel_vip = Ipv4Addr::UNSPECIFIED;
        let sentinel_vip_port: u16 = 0;

        self.reverse_local_map
            .upsert(
                sentinel_backend_ip,
                sentinel_backend_port,
                sentinel_proto,
                sentinel_vip,
                sentinel_vip_port,
            )
            .map_err(|e| DataplaneError::ReverseLocalProbe {
                message: format!("REVERSE_LOCAL_MAP sentinel insert: {e}"),
            })?;

        let sentinel_key = ReverseLocalKeyPod::from_typed(BackendKey::new(
            sentinel_backend_ip,
            sentinel_backend_port,
            sentinel_proto,
        ));
        let sentinel_value = ReverseLocalMapValue::encode(sentinel_vip, sentinel_vip_port);

        // Read-back via the dump surface, then verify the round-trip.
        // The sentinel key MUST be present with the sentinel VIP value;
        // a miss is the structural signal the reverse path is not
        // programmable on this kernel. BOTH non-confirmation paths — a
        // read-back failure AND a sentinel-absent miss — route through
        // one fallible verdict so they share the single cleanup site
        // below. The sentinel MUST be removed before the probe returns
        // on either: a stranded `(0.0.0.0:0, Udp)` sentinel is
        // inconsistent map state for the process lifetime (the leak this
        // guards against — the earlier inline `entries()?` propagated a
        // read-back error without cleanup).
        if let Err(e) = reverse_local_sentinel_verdict(
            self.reverse_local_map.entries(),
            sentinel_key,
            sentinel_value,
        ) {
            // Best-effort cleanup before bailing; the verdict failure is
            // the real story so a delete failure here is swallowed.
            let _ = self.reverse_local_map.remove(
                sentinel_backend_ip,
                sentinel_backend_port,
                sentinel_proto,
            );
            return Err(e);
        }

        self.reverse_local_map
            .remove(sentinel_backend_ip, sentinel_backend_port, sentinel_proto)
            .map_err(|e| DataplaneError::ReverseLocalProbe {
            message: format!("REVERSE_LOCAL_MAP sentinel delete: {e}"),
        })?;

        Ok(())
    }

    /// Public read-back surface for `LOCAL_BACKEND_MAP` — used by
    /// the walking-skeleton test to assert the cgroup_sock_addr
    /// path was populated. Mirrors the existing
    /// [`Self::backend_map_entries`] inspector shape.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] when the underlying
    /// map iteration fails.
    pub fn local_backend_map_entries(
        &self,
    ) -> Result<Vec<(crate::maps::LocalServiceKey, crate::maps::LocalBackendEntry)>, DataplaneError>
    {
        self.local_backend_map
            .entries()
            .map_err(|e| DataplaneError::LoadFailed(format!("LOCAL_BACKEND_MAP iter: {e}")))
    }

    /// Public read-back surface for `REVERSE_LOCAL_MAP` (ADR-0053 rev
    /// 2026-06-05, GH #200) — the `bpftool map dump`-equivalent the
    /// Tier-3 acceptance asserts on (S-01-02): after one
    /// `register_local_backend`, the reverse map carries
    /// `(backend_ip, backend_port, proto) → vip`. Mirrors
    /// [`Self::local_backend_map_entries`].
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] when the underlying map
    /// iteration fails.
    pub fn reverse_local_map_entries(
        &self,
    ) -> Result<
        Vec<(
            crate::maps::reverse_local_map_handle::ReverseLocalKeyPod,
            crate::maps::reverse_local_map_handle::ReverseLocalEntryPod,
        )>,
        DataplaneError,
    > {
        self.reverse_local_map
            .entries()
            .map_err(|e| DataplaneError::LoadFailed(format!("REVERSE_LOCAL_MAP iter: {e}")))
    }

    /// Read-back surface for `REVERSE_LOCAL_MISS_COUNTER` (ADR-0053 § D3
    /// rev 2026-06-05b / UI-1, GH #200) — the percpu-array dump the
    /// Tier-3 hardening test asserts on (S-03-01 assertion c). Returns
    /// the count of `cgroup_recvmsg4_service` `REVERSE_LOCAL_MAP` misses
    /// summed across every online CPU (the `aggregate_per_cpu` fold the
    /// `DROP_COUNTER` read path uses).
    ///
    /// A miss is the common non-service-reply case — recvmsg4 fires on
    /// ALL subtree unconnected UDP, and only a registered backend's reply
    /// hits. The counter is observable-but-inert: its incrementing has no
    /// effect on the source the app reads (the miss path is a pure no-op,
    /// not a sentinel rewrite).
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] when the underlying per-CPU
    /// slot read fails.
    pub fn reverse_local_miss_count(&self) -> Result<u64, DataplaneError> {
        // Single-slot counter; slot 0 is the reverse-miss count
        // (lockstep with `reverse_local_miss_counter.rs`'s
        // `SLOT_REVERSE_MISS` / `MAX_ENTRIES = 1`).
        const SLOT_REVERSE_MISS: u32 = 0;
        let per_cpu = self.reverse_local_miss_counter.get(&SLOT_REVERSE_MISS, 0).map_err(|e| {
            DataplaneError::LoadFailed(format!(
                "REVERSE_LOCAL_MISS_COUNTER PerCpuArray::get(slot={SLOT_REVERSE_MISS}): {e}"
            ))
        })?;
        Ok(overdrive_core::dataplane::aggregate_per_cpu(&per_cpu))
    }

    /// Returns the LOCAL_BACKEND_MAP entry for `(vip, vip_port)`,
    /// if any. Used by the walking-skeleton test to assert the
    /// cgroup path was populated by the hydrator's
    /// `Action::RegisterLocalBackend` dispatch.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] on lookup failure
    /// other than `KeyNotFound` (which surfaces as `Ok(None)`).
    pub fn local_backend_for(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<Option<crate::maps::LocalBackendEntry>, DataplaneError> {
        self.local_backend_map
            .get(vip, vip_port, proto)
            .map_err(|e| DataplaneError::LoadFailed(format!("LOCAL_BACKEND_MAP get: {e}")))
    }

    /// Test-only seam: arm the probe-fault. The next call to
    /// [`Self::probe`] consumes the armed fault and returns it
    /// verbatim BEFORE touching the kernel. Used by S-BDB-14 to
    /// exercise the `DataplaneBootError::Probe` mapping arm without
    /// corrupting the real BACKEND_MAP.
    ///
    /// Takes `&self` because the seam mutates only the interior
    /// `parking_lot::Mutex` — no exclusive borrow required. This
    /// matches the rest of `EbpfDataplane`'s mutating surface
    /// (`update_service` et al. take `&self` and route through
    /// interior `Mutex`/`Atomic`s).
    ///
    /// Gated behind `#[cfg(any(test, feature = "integration-tests"))]`
    /// — the symbol does not exist in production builds.
    #[cfg(any(test, feature = "integration-tests"))]
    pub fn set_probe_fault(&self, fault: DataplaneError) {
        *self.probe_fault.lock() = Some(fault);
    }
}

/// Graceful-shutdown RAII per architecture.md § 5.6 of
/// `backend-discovery-bridge-service-reachability` (step 02-02).
///
/// On drop:
///   - `_xdp_forward_link` / `_xdp_reverse_link` `XdpLinkId` fields
///     drop in field declaration order, and aya detaches each XDP
///     program from its iface as part of their `Drop`. No explicit
///     action required here.
///   - The SERVICE_MAP bpffs pin at `<pin_dir>/SERVICE_MAP` is
///     unlinked best-effort. Failure on `remove_file` logs at debug
///     — by the time `Drop` runs the caller may be unwinding from a
///     panic, and `Drop` cannot bubble errors. The leftover-pin
///     cleanup discipline in `.claude/rules/debugging.md` § "Leftover
///     XDP attachments across runs" is the operator-side safety net
///     when `Drop` is skipped (SIGKILL).
///
/// `NotFound` is treated as success (a prior unclean shutdown plus
/// the cleanup-on-start logic in `new_with_pin_dir` can leave the
/// pin gone before `Drop` runs).
impl Drop for EbpfDataplane {
    fn drop(&mut self) {
        let pin_path = self.pin_dir.join(SERVICE_MAP_NAME);
        if let Err(e) = std::fs::remove_file(&pin_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!(
                    name: "xdp.shutdown.unlink_failed",
                    path = %pin_path.display(),
                    error = %e,
                    "SERVICE_MAP pin unlink failed during shutdown"
                );
            }
        }
        // `XdpLinkId` fields held by `self` drop here; aya detaches
        // each XDP program from its iface as part of `XdpLinkId::Drop`.
    }
}

/// Classify an `io::Error` from `aya::programs::Xdp::attach` (which
/// surfaces as `ProgramError::SyscallError { call: "bpf_link_create"
/// | "netlink_set_xdp_fd", io_error }`) into either "fall back to
/// generic" or "propagate as-is". The classification is deliberately
/// narrow: only the documented driver-not-supported errno code
/// (`EOPNOTSUPP`, which on Linux is the SAME numeric value as
/// `ENOTSUP` — both `= 95`; POSIX names them distinctly but the libc
/// crate exposes them as aliases on the linux target) triggers
/// fallback. Everything else — `EINVAL` (often genuinely-invalid
/// attempts), `EPERM` (capability failure), `EBUSY`
/// (already-attached), errors without an OS errno — propagates as
/// `DataplaneError::LoadFailed`. Falling back on an ambiguous error
/// would mask real loader bugs (per `.claude/rules/development.md`
/// § Errors — distinct failure modes get distinct variants).
///
/// **Single equality check**: a previous shape compared against both
/// `libc::EOPNOTSUPP` AND `libc::ENOTSUP` joined by `||`. On Linux
/// that comparison is structurally redundant — the two constants are
/// numerically identical — so the boolean operator (`||` or `&&`)
/// was never observable, which is precisely the situation `cargo
/// mutants` flagged with an unkillable `||→&&` mutation. Collapsing
/// to a single comparison removes the operator entirely; a future
/// kernel header change that drifts the two apart would surface as a
/// libc release that breaks the equivalence (see the paired unit
/// test below pinning `EOPNOTSUPP == ENOTSUP`).
///
/// Lives at module scope rather than as an inherent method so the
/// unit tests in `mod tests` below can exercise it without
/// constructing a full `EbpfDataplane`. Keeps the fallback decision
/// pure-function-shaped — same property the wider DST harness relies
/// on for replay equivalence.
fn should_fallback_to_generic(io_error: &std::io::Error) -> bool {
    io_error.raw_os_error().is_some_and(|code| code == libc::EOPNOTSUPP)
}

/// Select the `SERVICE_MAP` outer-map slots an empty-backend
/// `update_service(frontend, [])` must purge.
///
/// The purge is scoped to the **single frontend identity**
/// `(vip, port, proto)` — exactly the [`crate::maps::ServiceKey`] the
/// non-empty path keys on. A co-resident listener on the same VIP under
/// a different `(port, proto)` — e.g. CoreDNS `udp/53` alongside
/// `tcp/53`, installed by a *separate* per-listener `update_service`
/// call — keeps its outer-map slot, its `BACKEND_MAP` entries, and its
/// `REVERSE_NAT_MAP` keys. This is the **per-proto purge** the
/// [`Dataplane`] trait contract mandates (ADR-0060 D4; the trait
/// docstring in `overdrive-core`: *"REVERSE_NAT keys for other
/// protocols of the same VIP are not removed"*), and the behaviour
/// `SimDataplane::update_service` already implements.
///
/// Selecting by `vip_host` alone is the bug this guards against: it
/// would pick every listener registered on the VIP, transiently
/// deleting a live listener's outer slot so XDP `XDP_PASS`es packets it
/// should forward. The selected set then drives all three cleanup
/// passes (outer-slot delete, `BACKEND_MAP` orphan-GC, `REVERSE_NAT`
/// purge), so an over-broad selection over-purges every map.
///
/// Lives at module scope rather than inline in `update_service` so the
/// unit tests in `mod tests` can exercise the selection without
/// constructing a full `EbpfDataplane` (mirrors
/// [`should_fallback_to_generic`]).
fn empty_backend_purge_keys<'a>(
    tracked_keys: impl Iterator<Item = &'a crate::maps::ServiceKey>,
    frontend_key: crate::maps::ServiceKey,
) -> Vec<crate::maps::ServiceKey> {
    tracked_keys
        .filter(|k| {
            k.vip_host == frontend_key.vip_host
                && k.port_host == frontend_key.port_host
                && k.proto == frontend_key.proto
        })
        .copied()
        .collect()
}

/// Read-back verdict for the `REVERSE_LOCAL_MAP` sentinel round-trip
/// (ADR-0053 rev D5b/c, GH #200).
///
/// `Ok(())` means the dumped `entries` contain the sentinel key with
/// the expected VIP — the reverse path is programmable and
/// [`EbpfDataplane::probe_reverse_local`] proceeds to delete the
/// sentinel. `Err(_)` means the read-back itself failed OR the sentinel
/// was absent; in **both** cases the caller MUST remove the sentinel
/// before returning, so the two non-confirmation paths share one
/// cleanup site.
///
/// Folding the read-back-failure and round-trip-miss branches into a
/// single fallible verdict is what closes the sentinel-leak gap: the
/// earlier inline `entries()?` propagated a read error WITHOUT removing
/// the sentinel, stranding the reserved `(0.0.0.0:0, Udp) → 0.0.0.0`
/// entry in the map for the process lifetime.
///
/// Lives at module scope rather than inline in `probe_reverse_local` so
/// the unit tests in `mod tests` can exercise the verdict without
/// constructing a full `EbpfDataplane` (mirrors
/// [`empty_backend_purge_keys`] / [`should_fallback_to_generic`]) —
/// the probe itself needs a real kernel map and has no Tier-2 backstop.
fn reverse_local_sentinel_verdict(
    entries: Result<
        Vec<(
            crate::maps::reverse_local_map_handle::ReverseLocalKeyPod,
            crate::maps::reverse_local_map_handle::ReverseLocalEntryPod,
        )>,
        aya::maps::MapError,
    >,
    sentinel_key: crate::maps::reverse_local_map_handle::ReverseLocalKeyPod,
    sentinel_value: crate::maps::reverse_local_map_handle::ReverseLocalEntryPod,
) -> Result<(), DataplaneError> {
    let entries = entries.map_err(|e| DataplaneError::ReverseLocalProbe {
        message: format!("REVERSE_LOCAL_MAP sentinel read-back: {e}"),
    })?;
    if entries.iter().any(|(k, v)| *k == sentinel_key && *v == sentinel_value) {
        Ok(())
    } else {
        Err(DataplaneError::ReverseLocalProbe {
            message: "REVERSE_LOCAL_MAP sentinel round-trip missed: key absent after insert"
                .to_string(),
        })
    }
}

/// Verdict from classifying an `aya::programs::Xdp::attach` result
/// against the native→generic fallback policy. Wraps the three
/// outcomes the loader's two attach call sites (forward-path on
/// `client_iface`, reverse-path on `backend_iface`) need to
/// distinguish:
///
/// - [`AttachOutcome::Native`] — `DRV_MODE` succeeded; the link is
///   live on the NIC's native XDP hook.
/// - [`AttachOutcome::Fallback`] — `DRV_MODE` returned a `SyscallError`
///   whose `io_error` is `EOPNOTSUPP`/`ENOTSUP`; the caller emits the
///   structured `xdp.attach.fallback_generic` warn and retries with
///   `SKB_MODE`. The `syscall` field carries the failing syscall name
///   (`"bpf_link_create"` or `"netlink_set_xdp_fd"`) for the warn
///   payload.
/// - [`AttachOutcome::Propagate`] — every other `ProgramError`
///   variant (genuine `EINVAL`, `EPERM`, `EBUSY`, non-syscall errors,
///   syscall errors without an `EOPNOTSUPP` errno). Falling back on
///   these would mask real loader bugs per
///   `.claude/rules/development.md` § Errors.
///
/// Lifting the match guard out of the call site into this typed
/// classifier is what makes the policy mutation-killable: Lima
/// virtio-net supports native XDP, so the in-VM Tier 3 attach path
/// never exercises the fallback arm — but the unit tests below DO,
/// against synthetic `ProgramError::SyscallError` values constructed
/// from arbitrary `io::Error` shapes. Mutating the fallback predicate
/// (e.g. `code == EOPNOTSUPP` → `false`) flips the EOPNOTSUPP test to
/// `Propagate`; mutating to `true` flips the EINVAL test to
/// `Fallback`. Each mutation is killable.
#[derive(Debug)]
enum AttachOutcome<L> {
    Native(L),
    Fallback { syscall: &'static str },
    Propagate(aya::programs::ProgramError),
}

/// Classify the result of `aya::programs::Xdp::attach(iface, DRV_MODE)`
/// against the project's native→generic fallback policy. See
/// [`AttachOutcome`] for the three verdict variants.
///
/// This helper is the single source of truth for the fallback
/// predicate; both forward-path and reverse-path call sites in
/// [`EbpfDataplane::new_with_pin_dir`] consume its output. Keeping
/// the classifier pure-function-shaped (no I/O, no logging, no
/// `prog: &mut Xdp` dependency) means the unit tests can drive every
/// arm without standing up a real BPF program — the ~15 ms warm
/// inner loop the §21 DST harness relies on.
fn classify_attach_result<L>(result: Result<L, aya::programs::ProgramError>) -> AttachOutcome<L> {
    use aya::programs::ProgramError;

    match result {
        Ok(link) => AttachOutcome::Native(link),
        Err(ProgramError::SyscallError(ref se)) if should_fallback_to_generic(&se.io_error) => {
            AttachOutcome::Fallback { syscall: se.call }
        }
        Err(e) => AttachOutcome::Propagate(e),
    }
}

/// Classify an `aya::programs::Xdp::attach` error into the honest
/// `EBUSY`-slot-collision diagnostic (ADR-0061 § 5 / D3), or `None`
/// when the error is something else.
///
/// The kernel permits exactly one XDP program per netdev XDP hook;
/// attaching a second program to an occupied hook returns `EBUSY`.
/// The single-node default (`DataplaneConfig::single_node_veth()`)
/// gives the forward (`client_iface`) and reverse (`backend_iface`)
/// programs two distinct veth names, so the collision is structurally
/// unreachable on the default path (ADR-0061 § 1). An operator who
/// points both ifaces at one real NIC still hits it.
///
/// Returns `Some(DataplaneError::IfaceXdpSlotBusy { iface })` only when
/// the attach error is a `SyscallError` whose `io_error` carries the
/// `EBUSY` errno; every other error (`EOPNOTSUPP` fallback, `EPERM`,
/// `ENODEV`, non-syscall errors) returns `None` so the caller falls
/// through to its existing `LoadFailed`/`DRV_MODE` handling. This is
/// the same `raw_os_error()` reach `should_fallback_to_generic` uses,
/// swapping `EOPNOTSUPP` for `libc::EBUSY` — and the same
/// pure-function shape, so the default-lane unit tests below drive it
/// against synthetic `SyscallError` values without a real attach (Lima
/// virtio-net never produces a real `EBUSY` here; 01-04's Tier 3 path
/// does).
fn classify_iface_xdp_slot_busy(
    error: &aya::programs::ProgramError,
    iface: &str,
) -> Option<DataplaneError> {
    use aya::programs::ProgramError;

    match error {
        ProgramError::SyscallError(se) if se.io_error.raw_os_error() == Some(libc::EBUSY) => {
            Some(DataplaneError::IfaceXdpSlotBusy { iface: iface.to_owned() })
        }
        _ => None,
    }
}

/// Build the typed [`DataplaneError::CgroupSendRecvAttach`] for a failed
/// load/attach of an unconnected-UDP cgroup hook (ADR-0053 D5b/c, GH
/// #200).
///
/// The aya `ProgramError` from `load()`/`attach()` is classified into the
/// typed variant: when the failure carries a `SyscallError` we extract its
/// `io_error` (preserving the `EOPNOTSUPP`/`ENOSYS` errno that a
/// below-floor kernel returns); otherwise we synthesise an `io::Error`
/// from the `ProgramError`'s `Display` so the `source` chain is never
/// empty. Either way the result is the typed variant the composition root
/// routes to `health.startup.refused` — NEVER a flattened
/// `LoadFailed(String)` (`.claude/rules/development.md` § "Never flatten a
/// typed error"). Mirrors the `classify_iface_xdp_slot_busy` precedent.
fn cgroup_attach_error(hook: &'static str, error: &aya::programs::ProgramError) -> DataplaneError {
    use aya::programs::ProgramError;

    let source = match error {
        ProgramError::SyscallError(se) => se.io_error.raw_os_error().map_or_else(
            || std::io::Error::other(error.to_string()),
            std::io::Error::from_raw_os_error,
        ),
        other => std::io::Error::other(other.to_string()),
    };
    DataplaneError::CgroupSendRecvAttach { hook, source }
}

/// Build the typed [`DataplaneError::LoadFailed`] for a failed
/// `load()` of an unconnected-UDP cgroup hook. A load failure is a
/// BPF program-correctness problem (verifier rejection, missing
/// program, capability gap) — NOT the below-floor attach refusal
/// `cgroup_attach_error` / `CgroupSendRecvAttach` is reserved for
/// (ADR-0053 D5b/c: the attach() syscall is the kernel-version
/// oracle, not load()). Mirrors the `cgroup_connect4_service.load`
/// inline mapping.
fn cgroup_load_error(hook: &'static str, error: &aya::programs::ProgramError) -> DataplaneError {
    DataplaneError::LoadFailed(format!("{hook}.load: {error}"))
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl Dataplane for EbpfDataplane {
    /// see #24 (`POLICY_MAP`)
    async fn update_policy(
        &self,
        _key: PolicyKey,
        _verdict: Verdict,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    /// 5-step atomic backend-set swap per ADR-0040 § 2.
    ///
    /// `significant_drop_tightening` is allowed at the fn level
    /// because the BACKEND_MAP lock is intentionally scoped to a
    /// `{ ... }` block — the lint wants an explicit `drop()` but
    /// the scope braces serve the same purpose more idiomatically.
    #[allow(clippy::significant_drop_tightening)]
    ///
    /// 1. Upsert each `Backend` into BACKEND_MAP under a `BackendId`
    ///    derived from the backend's `(IPv4, port)`.
    /// 2. Allocate a fresh inner ARRAY (size 256) populated with the
    ///    new backend slot table. On kernel rejection
    ///    (`EINVAL`/`EPERM`/`ENOMEM`) return
    ///    [`DataplaneError::MapAllocFailed`] without touching the
    ///    outer map (S-2.2-11 preservation invariant).
    /// 3. Single `bpf_map_update_elem` against SERVICE_MAP outer:
    ///    `set(&service_key, new_inner.as_fd())`. Kernel ref-counts
    ///    inner maps; concurrent XDP readers see either the old or
    ///    the new inner-map pointer atomically.
    /// 4. Orphan GC — sweep BACKEND_MAP for entries no longer in
    ///    the new set. Phase 2.2 step 03-02 ships a minimal sweep
    ///    bounded by the just-inserted set; the broader cross-
    ///    service GC (S-2.2-10) is a separate slice landing.
    /// 5. The old inner map's FD goes out of scope inside aya's
    ///    own ref-counting machinery — the kernel reaps the map
    ///    once no XDP program references it (refcount = 0).
    ///
    /// VIP port note: the VIP port for the SERVICE_MAP `ServiceKey`
    /// and the reverse-NAT `VipPod` value is sourced from
    /// `frontend.port()` — the `ServiceFrontend` triple already
    /// carries the declared `(vip, port, proto)`. It is NOT derived
    /// from `backends[0].addr.port()`: a backend's listener port may
    /// differ from the VIP port (e.g. VIP:53 → backend:5353), and the
    /// XDP program keys the outer slot on the packet's destination
    /// port (= the VIP port).
    async fn update_service(
        &self,
        frontend: overdrive_core::dataplane::ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        use std::os::fd::AsFd;

        use crate::maps::wire::{BackendEntryPod, ServiceKey};
        use crate::maps::{BackendKeyPod, VipPod};

        // `frontend.vip` is IPv4-by-construction (ADR-0060 D1a) — narrow
        // infallibly. `frontend.proto()` is the single declared L4
        // protocol; the REVERSE_NAT fan-out is per-proto (D4).
        let vip = frontend.vip_v4();
        let svc_proto = frontend.proto();

        // Empty backend set — remove THIS frontend's slot from all maps
        // so XDP returns XDP_PASS for it. The purge is scoped to the
        // single `(vip, port, proto)` identity (per-proto purge,
        // ADR-0060 D4): the same VIP may carry several listeners
        // installed by separate `update_service` calls (CoreDNS
        // `tcp/53` + `udp/53`), and an empty-backend update for one MUST
        // leave the others' outer-map slots intact. `frontend.port()`
        // is the listener port — `backends` is empty here, so the
        // non-empty path's `backends[0].addr.port()` is unavailable.
        if backends.is_empty() {
            let frontend_key = crate::maps::wire::ServiceKey {
                vip_host: u32::from(vip),
                port_host: frontend.port().get(),
                proto: svc_proto.as_u8(),
                _pad: 0,
            };

            let matching_keys: Vec<crate::maps::wire::ServiceKey> = {
                let tracker = self.service_backends.lock();
                empty_backend_purge_keys(tracker.keys(), frontend_key)
            };

            if matching_keys.is_empty() {
                return Ok(());
            }

            for service_key in &matching_keys {
                self.service_map.delete(service_key).map_err(|e| match e {
                    crate::maps::hash_of_maps::HashOfMapsError::MapAllocFailed { source } => {
                        DataplaneError::MapAllocFailed { source }
                    }
                    crate::maps::hash_of_maps::HashOfMapsError::Syscall(source) => {
                        DataplaneError::LoadFailed(format!("SERVICE_MAP outer delete: {source}"))
                    }
                })?;
            }

            let live_ids: std::collections::BTreeSet<u32> = {
                let mut tracker = self.service_backends.lock();
                for service_key in &matching_keys {
                    tracker.remove(service_key);
                }
                tracker
                    .values()
                    .flat_map(|s| s.iter().copied())
                    .collect::<std::collections::BTreeSet<u32>>()
            };
            {
                let mut backend_map = self.backend_map.lock();
                let removed = crate::gc::sweep_orphan_backends(&mut backend_map, &live_ids)
                    .map_err(|e| {
                        DataplaneError::LoadFailed(format!("BACKEND_MAP orphan-GC sweep: {e}"))
                    })?;
                if !removed.is_empty() {
                    let mut alloc = self.backend_id_alloc.lock();
                    for removed_id in &removed {
                        if let Ok(bid) = overdrive_core::id::BackendId::new(*removed_id) {
                            alloc.release(bid);
                        }
                    }
                }
            }

            let stale_keys: std::collections::BTreeSet<crate::maps::BackendKeyPod> = {
                let mut tracker = self.service_reverse_nat_keys.lock();
                let prior_keys: std::collections::BTreeSet<crate::maps::BackendKeyPod> =
                    matching_keys
                        .iter()
                        .flat_map(|sk| tracker.remove(sk).unwrap_or_default())
                        .collect();
                let live: std::collections::BTreeSet<crate::maps::BackendKeyPod> =
                    tracker.values().flat_map(|s| s.iter().copied()).collect();
                prior_keys.difference(&live).copied().collect()
            };
            {
                let mut reverse_nat_map = self.reverse_nat_map.lock();
                for stale in &stale_keys {
                    match reverse_nat_map.remove(stale) {
                        Ok(()) | Err(aya::maps::MapError::KeyNotFound) => {}
                        Err(e) => {
                            return Err(DataplaneError::LoadFailed(format!(
                                "REVERSE_NAT_MAP purge: {e}"
                            )));
                        }
                    }
                }
            }

            return Ok(());
        }

        let vip_port = frontend.port().get();
        // Step 02-01: proto sourced from the `ServiceFrontend` (ADR-0060
        // site #7) — NO `Proto::Tcp` literal on the action→handle→key
        // path, so a UDP service keys a distinct outer-map slot (C3
        // guard).
        let service_key = ServiceKey {
            vip_host: u32::from(vip),
            port_host: vip_port,
            proto: svc_proto.as_u8(),
            _pad: 0,
        };

        // Step 1 — Upsert each backend into BACKEND_MAP. BackendId
        // is assigned by the monotonic-counter allocator per ADR-0046.
        // Same (ip, port, proto) yields the same BackendId across
        // updates (memo-table hit); orphan GC removes IDs not in the
        // new set.
        //
        // Proto threaded from the `ServiceFrontend` (ADR-0060, GH #163)
        // — the per-service L4 protocol, no longer hardcoded TCP.
        let proto = svc_proto.as_u8();
        let mut backend_ids: Vec<u32> = Vec::with_capacity(backends.len());
        {
            // Locks held only for the BACKEND_MAP populate loop;
            // dropped at end of this block before any further work.
            let mut backend_map = self.backend_map.lock();
            let mut alloc = self.backend_id_alloc.lock();
            for backend in &backends {
                let pod = BackendEntryPod::from_backend(backend)?;
                let backend_id: u32 = alloc.allocate(pod.ipv4_host, pod.port_host, proto).get();
                backend_map
                    .insert(backend_id, pod, 0)
                    .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP insert: {e}")))?;
                backend_ids.push(backend_id);
            }
            // Lock dropped here, before any further work that could
            // .await (per `.claude/rules/development.md` § Concurrency:
            // never hold a lock across `.await`).
        }

        // Step 2 — Allocate a fresh inner ARRAY (size 256) and
        // populate slots with round-robin BackendIds. On alloc
        // rejection convert HashOfMapsError::MapAllocFailed →
        // DataplaneError::MapAllocFailed (the typed S-2.2-11 path).
        let new_inner = self.service_map.create_inner(None).map_err(|e| match e {
            crate::maps::hash_of_maps::HashOfMapsError::MapAllocFailed { source } => {
                DataplaneError::MapAllocFailed { source }
            }
            crate::maps::hash_of_maps::HashOfMapsError::Syscall(source) => {
                DataplaneError::LoadFailed(format!("inner-map alloc: {source}"))
            }
        })?;

        // Populate inner ARRAY slots via the Maglev permutation
        // (Slice 04 — replaces Slice 03's round-robin populate). The
        // permutation is a deterministic function of the weighted
        // backend set, ordered canonically by `BTreeMap<BackendId,
        // Weight>` per `.claude/rules/development.md` § Ordered-
        // collection choice; the same backend set produces the same
        // permutation byte-for-byte across runs and across nodes
        // (DST invariant `MaglevDeterministic`; S-2.2-12).
        //
        // Two structural properties matter at this seam:
        //
        // 1. **Distribution evenness** — each backend appears in
        //    ≈ M / N_backends slots; under uniformly hashed traffic
        //    each backend receives ≈ 1/N of the load (S-2.2-15
        //    bound: ±5 %).
        // 2. **Disruption bound** — adding or removing one backend
        //    shifts ≤ 1 / N_backends ≈ 1 % of slots (ASR-2.2-02).
        //    This is the consistent-hashing guarantee that makes
        //    backend-set churn cheap; without Maglev a flat hash
        //    would re-shuffle ~all slots on any change.
        //
        // The XDP fast path indexes this populated ARRAY by
        // FNV-1a(5-tuple) mod M — see
        // `crates/overdrive-bpf/src/programs/xdp_service_map.rs`.
        let weighted: std::collections::BTreeMap<overdrive_core::id::BackendId, u16> = backends
            .iter()
            .map(|backend| {
                let pod = BackendEntryPod::from_backend(backend)?;
                let bid =
                    self.backend_id_alloc.lock().allocate(pod.ipv4_host, pod.port_host, proto);
                Ok((bid, backend.weight.max(1)))
            })
            .collect::<Result<_, DataplaneError>>()?;
        let permutation = overdrive_core::maglev::permutation::generate(
            &weighted,
            overdrive_core::dataplane::MaglevTableSize::DEFAULT,
        );
        // Defensive: if `generate` returns a table with the wrong size
        // (only possible on empty inputs, which we short-circuited at
        // the top of this fn), fall back to LoadFailed rather than
        // silently mispopulating.
        if permutation.len() != SERVICE_MAP_INNER_CAPACITY as usize {
            return Err(DataplaneError::LoadFailed(format!(
                "maglev::generate returned {} slots; expected {}",
                permutation.len(),
                SERVICE_MAP_INNER_CAPACITY
            )));
        }
        for (slot, bid) in permutation.iter().enumerate() {
            // Slot is bounded by the permutation length check above
            // (SERVICE_MAP_INNER_CAPACITY = 16_381, well within u32);
            // the cast is provably lossless.
            #[allow(clippy::cast_possible_truncation)]
            let key_bytes = (slot as u32).to_ne_bytes();
            let value_bytes = bid.get().to_ne_bytes();
            crate::sys::bpf::bpf_map_update_elem(
                new_inner.as_fd(),
                &key_bytes,
                &value_bytes,
                crate::sys::bpf::BPF_ANY,
            )
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("inner-map slot {slot} populate: {e}"))
            })?;
        }

        // Step 2b — Pre-populate REVERSE_NAT_MAP for new backends.
        //
        // MUST land BEFORE the SERVICE_MAP swap (step 3) so the
        // reverse path is ready before any packet can be forwarded
        // to a new backend. Without this ordering, a response from
        // a newly-added backend arrives at xdp_reverse_nat_lookup,
        // misses the REVERSE_NAT_MAP, and escapes with the backend
        // IP as source — breaking the VIP abstraction.
        //
        // Inserting with BPF_ANY is idempotent: re-adding an
        // existing backend is a no-op update. Stale entries for
        // removed backends are purged in step 5 (after the swap),
        // which is safe because removed backends no longer receive
        // forward-path traffic.
        let new_keys: std::collections::BTreeSet<BackendKeyPod> = backends
            .iter()
            .filter_map(|backend| match backend.addr.ip() {
                std::net::IpAddr::V4(v4) => Some(BackendKeyPod {
                    ip_host: u32::from(v4),
                    port_host: backend.addr.port(),
                    proto: svc_proto.as_u8(),
                    _pad: 0,
                }),
                std::net::IpAddr::V6(_) => None,
            })
            .collect();
        let vip_value = VipPod { ip_host: u32::from(vip), port_host: vip_port, _pad: 0 };
        {
            let mut reverse_nat_map = self.reverse_nat_map.lock();
            for key in &new_keys {
                reverse_nat_map.insert(key, vip_value, 0).map_err(|e| {
                    DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP insert: {e}"))
                })?;
            }
        }

        // Step 3 — Atomic outer-pointer update. Single
        // bpf_map_update_elem syscall; kernel ref-counts the new
        // inner map and the old; concurrent XDP readers see one or
        // the other atomically. THIS IS THE LOAD-BEARING STEP.
        self.service_map.set(&service_key, new_inner.as_fd()).map_err(|e| match e {
            crate::maps::hash_of_maps::HashOfMapsError::MapAllocFailed { source } => {
                DataplaneError::MapAllocFailed { source }
            }
            crate::maps::hash_of_maps::HashOfMapsError::Syscall(source) => {
                DataplaneError::LoadFailed(format!("SERVICE_MAP outer set: {source}"))
            }
        })?;

        // Step 4 — Orphan GC (S-2.2-10).
        //
        // Update the per-service tracker with this update's BackendId
        // set, compute the live-set union across every active service,
        // and sweep BACKEND_MAP for entries no longer referenced.
        // Without this, BACKEND_MAP fills monotonically as services
        // shrink — see `crate::gc` module docs for the full rationale.
        //
        // Two locks held briefly back-to-back: `service_backends` for
        // the tracker update + union, `backend_map` for the sweep.
        // Both critical sections are pure-syscall — no `.await`
        // between acquire and release.
        let live_ids: std::collections::BTreeSet<u32> = {
            let mut tracker = self.service_backends.lock();
            tracker.insert(service_key, backend_ids.iter().copied().collect());
            tracker
                .values()
                .flat_map(|s| s.iter().copied())
                .collect::<std::collections::BTreeSet<u32>>()
        };
        {
            let mut backend_map = self.backend_map.lock();
            let removed =
                crate::gc::sweep_orphan_backends(&mut backend_map, &live_ids).map_err(|e| {
                    DataplaneError::LoadFailed(format!("BACKEND_MAP orphan-GC sweep: {e}"))
                })?;
            // Release removed ids from the allocator memo table
            // (ADR-0046). Lock acquired after backend_map is done —
            // same brief-hold discipline.
            if !removed.is_empty() {
                let mut alloc = self.backend_id_alloc.lock();
                for removed_id in &removed {
                    if let Ok(bid) = overdrive_core::id::BackendId::new(*removed_id) {
                        alloc.release(bid);
                    }
                }
            }
        }

        // Step 5 — REVERSE_NAT_MAP stale-entry purge (S-2.2-18).
        //
        // New entries already landed in step 2b (before the
        // SERVICE_MAP swap). This step purges entries for backends
        // that left this service and are not referenced by any
        // other active service. Runs after the swap because stale
        // reverse-NAT entries are harmless (removed backends no
        // longer receive forward-path traffic) while the pre-swap
        // insert ordering is safety-critical.
        //
        // The proto is `frontend.proto()` (ADR-0060 D4) — the
        // per-service L4 protocol threaded through `new_keys` above.
        let (prior_keys, live_nat_keys): (
            std::collections::BTreeSet<BackendKeyPod>,
            std::collections::BTreeSet<BackendKeyPod>,
        ) = {
            let mut tracker = self.service_reverse_nat_keys.lock();
            let prior = tracker.get(&service_key).cloned().unwrap_or_default();
            tracker.insert(service_key, new_keys.clone());
            let live = tracker.values().flat_map(|s| s.iter().copied()).collect();
            (prior, live)
        };
        {
            let mut reverse_nat_map = self.reverse_nat_map.lock();
            for stale in prior_keys.difference(&new_keys) {
                if live_nat_keys.contains(stale) {
                    continue;
                }
                match reverse_nat_map.remove(stale) {
                    Ok(()) | Err(aya::maps::MapError::KeyNotFound) => {}
                    Err(e) => {
                        return Err(DataplaneError::LoadFailed(format!(
                            "REVERSE_NAT_MAP purge: {e}"
                        )));
                    }
                }
            }
        }

        // Step 6 — Old inner map released by aya's ref-counting once
        // it goes out of scope in the kernel-side program tail. The
        // userspace `OwnedFd` we used to populate the new inner map
        // (`new_inner`) drops here, decrementing the userspace-side
        // refcount. The kernel keeps the inner map alive while
        // SERVICE_MAP outer references it.

        Ok(())
    }

    /// see #27 (telemetry `ringbuf`)
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(vec![])
    }

    /// ADR-0053 rev 2026-06-05 (GH #200) — register or replace the
    /// local backend for `(vip, vip_port, proto)` with the ORDERED
    /// reverse-first dual-write (DDD-1).
    ///
    /// Two BPF map syscalls, reverse BEFORE forward:
    /// 1. `REVERSE_LOCAL_MAP[(backend_ip, backend_port, proto)] =
    ///    (vip, vip_port)` — the reply store the
    ///    `cgroup_recvmsg4_service` program reads to rewrite the
    ///    unconnected reply source backend→VIP (BOTH address and port,
    ///    ADR-0053 §D4).
    /// 2. `LOCAL_BACKEND_MAP[(vip, vip_port, proto)] = backend`
    ///    — the forward store `cgroup_connect4`/`sendmsg4` read to
    ///    rewrite the destination VIP→backend.
    ///
    /// The order is an ORDERING guarantee, not atomicity (DDD-1, F-2):
    /// because the reverse entry lands first, no observer ever sees a
    /// forward entry without its matching reverse. A `sendmsg` the
    /// forward entry rewrites is guaranteed the reply's reverse entry
    /// is already present.
    async fn register_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: std::net::SocketAddrV4,
        proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        // Reverse FIRST (DDD-1) — keyed on the backend identity →
        // (vip, vip_port). The VIP port is restored alongside the VIP
        // address on the reply rewrite (ADR-0053 §D4).
        self.reverse_local_map
            .upsert(*backend.ip(), backend.port(), proto, vip, vip_port)
            .map_err(|e| DataplaneError::LocalBackendInsert {
                source: std::io::Error::other(format!(
                    "aya HashMap::insert (REVERSE_LOCAL_MAP): {e}"
                )),
            })?;
        // Forward SECOND — keyed on (vip, vip_port, proto) → backend.
        self.local_backend_map.upsert(vip, vip_port, backend, proto).map_err(|e| {
            DataplaneError::LocalBackendInsert {
                source: std::io::Error::other(format!(
                    "aya HashMap::insert (LOCAL_BACKEND_MAP): {e}"
                )),
            }
        })
    }

    /// ADR-0053 rev 2026-06-05 (GH #200) — idempotent dual-removal
    /// (DDD-5a). Removes BOTH the forward `(vip, vip_port, proto)`
    /// entry and its matching reverse `(backend, proto) → vip` entry.
    ///
    /// The signature carries only `(vip, vip_port, proto)`, so the
    /// backend identity needed for the reverse key is resolved by
    /// reading the forward entry first. Removal order is forward THEN
    /// reverse — so no `connect`/`sendmsg` can be rewritten toward a
    /// backend whose reverse entry is already gone (the
    /// no-forward-without-reverse invariant, inverted for teardown).
    /// A forward entry that was already absent removes nothing on
    /// either side. `KeyNotFound` is swallowed inside the typed
    /// handles per the trait contract.
    async fn deregister_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        // Resolve the backend identity from the forward entry BEFORE
        // removing it — the reverse key needs the backend addr/port.
        let backend_entry = self.local_backend_map.get(vip, vip_port, proto).map_err(|e| {
            DataplaneError::LocalBackendDelete {
                source: std::io::Error::other(format!("aya HashMap::get (LOCAL_BACKEND_MAP): {e}")),
            }
        })?;

        // Forward FIRST (DDD-5a teardown order). Idempotent.
        self.local_backend_map.remove(vip, vip_port, proto).map_err(|e| {
            DataplaneError::LocalBackendDelete {
                source: std::io::Error::other(format!(
                    "aya HashMap::remove (LOCAL_BACKEND_MAP): {e}"
                )),
            }
        })?;

        // Reverse SECOND — only when the forward entry existed (otherwise
        // we have no backend identity to key on, and there is nothing to
        // remove). Idempotent inside the handle.
        if let Some(entry) = backend_entry {
            let backend_ip = Ipv4Addr::from(entry.backend_ip_host);
            self.reverse_local_map.remove(backend_ip, entry.backend_port_host, proto).map_err(
                |e| DataplaneError::LocalBackendDelete {
                    source: std::io::Error::other(format!(
                        "aya HashMap::remove (REVERSE_LOCAL_MAP): {e}"
                    )),
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the native→generic fallback classification
    //! helper (S-2.2-02) and the `classify_attach_result` dispatcher.

    use super::DataplaneError;

    /// Regression — empty-backend purge must be scoped to the single
    /// frontend identity `(vip, port, proto)`, NOT VIP-wide.
    ///
    /// Reproduces the defect where `update_service(frontend, [])`
    /// selected every `ServiceKey` matching only `vip_host`, so an
    /// empty-backend update for one listener (`tcp/53`) deleted the
    /// co-resident `udp/53` outer-map slot — and cascaded the
    /// `BACKEND_MAP`/`REVERSE_NAT` purge — for every other listener on
    /// the same VIP. The `Dataplane` trait contract mandates a
    /// per-`(vip, port, proto)` purge (ADR-0060 D4); `SimDataplane`
    /// already honours it, so the buggy `EbpfDataplane` diverged from
    /// the sim and from the documented contract.
    ///
    /// The tracker below carries four co-resident slots: the target
    /// `tcp/53`, a same-VIP+port other-proto `udp/53`, a same-VIP+proto
    /// other-port `tcp/853`, and a different VIP entirely. An
    /// empty-backend update for `tcp/53` must select ONLY the `tcp/53`
    /// slot; the other three survive.
    #[test]
    fn empty_backend_purge_is_scoped_to_frontend_not_vip_wide() {
        use std::net::Ipv4Addr;

        use crate::maps::ServiceKey;

        // IANA L4 proto bytes (`Proto::as_u8`): TCP = 6, UDP = 17.
        const TCP: u8 = 6;
        const UDP: u8 = 17;

        let vip = u32::from(Ipv4Addr::new(10, 0, 0, 1));
        let target = ServiceKey { vip_host: vip, port_host: 53, proto: TCP, _pad: 0 };
        let other_proto = ServiceKey { vip_host: vip, port_host: 53, proto: UDP, _pad: 0 };
        let other_port = ServiceKey { vip_host: vip, port_host: 853, proto: TCP, _pad: 0 };
        let other_vip = ServiceKey {
            vip_host: u32::from(Ipv4Addr::new(10, 0, 0, 2)),
            port_host: 53,
            proto: TCP,
            _pad: 0,
        };

        let tracked = [target, other_proto, other_port, other_vip];

        let purge = super::empty_backend_purge_keys(tracked.iter(), target);

        assert_eq!(
            purge,
            vec![target],
            "empty-backend purge must select ONLY the frontend's own \
             (vip, port, proto) slot — co-resident listeners differing by \
             proto or port, and other VIPs, must survive"
        );
    }

    /// Regression — a `REVERSE_LOCAL_MAP` sentinel read-back FAILURE
    /// must be classified as a probe failure (`Err`), the same verdict
    /// as a round-trip miss, so `probe_reverse_local` removes the
    /// sentinel before returning. The earlier inline `entries()?`
    /// propagated a read-back error WITHOUT removing the sentinel,
    /// stranding the reserved `(0.0.0.0:0, Udp) → 0.0.0.0` entry in
    /// `REVERSE_LOCAL_MAP` for the process lifetime (ADR-0053 rev
    /// D5b/c, GH #200). Folding the read-back-failure and round-trip
    /// -miss branches into one fallible verdict gives both a single
    /// cleanup site at the call site.
    ///
    /// Exercises the pure verdict without constructing a full
    /// `EbpfDataplane` (mirrors
    /// `empty_backend_purge_is_scoped_to_frontend_not_vip_wide`) —
    /// `probe_reverse_local` itself needs a real kernel map and has no
    /// Tier-2 backstop, so the verdict seam is the default-lane guard.
    #[test]
    fn reverse_local_sentinel_verdict_pins_all_three_branches() {
        use std::net::Ipv4Addr;

        use aya::maps::MapError;
        use overdrive_core::dataplane::backend_key::{BackendKey, Proto};

        use crate::maps::reverse_local_map_handle::{ReverseLocalKeyPod, ReverseLocalMapValue};

        let sentinel_key =
            ReverseLocalKeyPod::from_typed(BackendKey::new(Ipv4Addr::UNSPECIFIED, 0, Proto::Udp));
        let sentinel_value = ReverseLocalMapValue::encode(Ipv4Addr::UNSPECIFIED, 0);

        // 1. Read-back FAILURE — THE regression. The map dump errored;
        //    the verdict must be `Err` so the caller cleans up the
        //    sentinel rather than leaking it.
        let read_back_failed = super::reverse_local_sentinel_verdict(
            Err(MapError::KeyNotFound),
            sentinel_key,
            sentinel_value,
        );
        assert!(
            read_back_failed.is_err(),
            "a read-back failure must be a probe failure so the sentinel is cleaned up"
        );

        // 2. Round-trip MISS — sentinel absent from a successful dump.
        //    Also `Err` (already-correct path; pinned so the two
        //    non-confirmation branches stay unified).
        let missed =
            super::reverse_local_sentinel_verdict(Ok(Vec::new()), sentinel_key, sentinel_value);
        assert!(missed.is_err(), "an absent sentinel must be a probe failure");

        // 3. CONFIRMED — sentinel present with the expected VIP. The
        //    only `Ok` path; the caller proceeds to delete the sentinel.
        let confirmed = super::reverse_local_sentinel_verdict(
            Ok(vec![(sentinel_key, sentinel_value)]),
            sentinel_key,
            sentinel_value,
        );
        assert!(
            confirmed.is_ok(),
            "a present sentinel with the expected VIP confirms the round-trip"
        );
    }

    /// Classification — `EOPNOTSUPP` from `bpf_link_create` /
    /// `netlink_set_xdp_fd` is the canonical "driver does not
    /// support native XDP" signal. Trigger fallback to generic
    /// (`SKB_MODE`).
    #[test]
    fn fallback_classification_eopnotsupp_yields_true() {
        use std::io;
        let err = io::Error::from_raw_os_error(libc::EOPNOTSUPP);
        assert!(super::should_fallback_to_generic(&err));
    }

    /// `ENOTSUP` — on Linux this is the same numeric value as
    /// `EOPNOTSUPP` (both = 95). POSIX names them distinctly but
    /// the libc crate exposes them as identical constants on the
    /// linux target. The pinned `assert_eq!` below makes that
    /// equivalence explicit at test time: a future kernel header
    /// change (or libc bump) that drifts them apart would fire this
    /// assertion before the second one ever ran, surfacing as a
    /// libc / glibc semantic break rather than a silent fallback
    /// regression. The simpler single-comparison shape of
    /// `should_fallback_to_generic` (one `code == EOPNOTSUPP`) relies
    /// on this equivalence to keep `ENOTSUP` falling back.
    #[test]
    fn fallback_classification_enotsup_yields_true() {
        use std::io;
        // Pin the platform invariant the simplified
        // `should_fallback_to_generic` relies on.
        assert_eq!(
            libc::EOPNOTSUPP,
            libc::ENOTSUP,
            "Linux libc must expose EOPNOTSUPP == ENOTSUP for the simplified \
             single-comparison fallback predicate to cover both spellings"
        );
        let err = io::Error::from_raw_os_error(libc::ENOTSUP);
        assert!(super::should_fallback_to_generic(&err));
    }

    /// `EINVAL` is ambiguous — drivers and the verifier both surface
    /// it for genuinely-invalid attempts (bad flags, bad program
    /// type, bad ifindex, etc). Falling back on `EINVAL` would mask
    /// real loader bugs, per `.claude/rules/development.md` § Errors
    /// (distinct failure modes get distinct variants). Must NOT
    /// trigger fallback.
    #[test]
    fn fallback_classification_einval_yields_false() {
        use std::io;
        let err = io::Error::from_raw_os_error(libc::EINVAL);
        assert!(!super::should_fallback_to_generic(&err));
    }

    /// `EPERM` is a permissions failure (`CAP_NET_ADMIN` missing,
    /// LSM denial, sysctl lock). Falling back to generic does not
    /// fix the underlying problem and would emit a misleading warn.
    /// Must NOT trigger fallback.
    #[test]
    fn fallback_classification_eperm_yields_false() {
        use std::io;
        let err = io::Error::from_raw_os_error(libc::EPERM);
        assert!(!super::should_fallback_to_generic(&err));
    }

    /// Errors that don't carry a `raw_os_error` (synthetic
    /// `io::Error::other(...)` constructions, future error shapes)
    /// must NOT trigger fallback — same conservative rule as
    /// `EINVAL` / `EPERM`.
    #[test]
    fn fallback_classification_no_os_errno_yields_false() {
        use std::io;
        let err = io::Error::other("synthetic, no errno");
        assert!(!super::should_fallback_to_generic(&err));
    }

    // ----- classify_attach_result coverage -----
    //
    // The two attach call sites in `EbpfDataplane::new_with_pin_dir`
    // route through `classify_attach_result`. Lima virtio-net
    // supports native XDP (`DRV_MODE` always succeeds), so the
    // Tier 3 inner loop never exercises the Fallback or Propagate
    // arms. These unit tests close the gap by driving every arm
    // against synthetic `aya::programs::ProgramError::SyscallError`
    // values — same shape `aya::programs::Xdp::attach` would
    // surface on a non-virtio NIC, without standing up a real BPF
    // program.
    //
    // Mutation-killing pattern: each arm of `classify_attach_result`
    // is asserted on by a dedicated test. Mutating the match guard
    // (e.g. `should_fallback_to_generic` → `true`) flips the EINVAL
    // test from Propagate to Fallback; mutating to `false` flips the
    // EOPNOTSUPP test from Fallback to Propagate. The `Native(_)`
    // arm is independently asserted by the `Ok(())` test.

    /// `Ok(link)` from the underlying attach surfaces as
    /// [`AttachOutcome::Native`] with the link payload preserved
    /// verbatim. Drives the happy path without standing up a real
    /// XDP program; the link type is generic over `L`.
    #[test]
    fn classify_attach_result_ok_yields_native_with_link() {
        let outcome: super::AttachOutcome<u32> = super::classify_attach_result(Ok(42u32));
        match outcome {
            super::AttachOutcome::Native(link) => assert_eq!(link, 42),
            other => panic!("expected AttachOutcome::Native(42), got {other:?}"),
        }
    }

    /// `Err(SyscallError { io_error: EOPNOTSUPP, call:
    /// "bpf_link_create" })` surfaces as [`AttachOutcome::Fallback`]
    /// carrying the originating syscall name. This is the only error
    /// shape that should drive the SKB retry — the docstring on
    /// [`AttachOutcome::Fallback`] makes the policy explicit.
    #[test]
    fn classify_attach_result_eopnotsupp_yields_fallback_with_syscall_name() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_link_create",
            io_error: io::Error::from_raw_os_error(libc::EOPNOTSUPP),
        });
        let outcome: super::AttachOutcome<()> = super::classify_attach_result(Err(err));
        match outcome {
            super::AttachOutcome::Fallback { syscall } => {
                assert_eq!(syscall, "bpf_link_create");
            }
            other => panic!("expected AttachOutcome::Fallback, got {other:?}"),
        }
    }

    /// `Err(SyscallError { io_error: EINVAL, ... })` is ambiguous —
    /// the kernel surfaces it for genuinely-invalid attach attempts
    /// (bad flags, bad ifindex, verifier-rejected program) and
    /// falling back would mask real loader bugs. Must surface as
    /// [`AttachOutcome::Propagate`] so the caller wraps it as
    /// `DataplaneError::LoadFailed`.
    ///
    /// This pairs with the EOPNOTSUPP test above to kill the match
    /// guard mutants: flipping the predicate to `true` turns this
    /// case into Fallback (assertion fires); flipping to `false`
    /// turns the EOPNOTSUPP case into Propagate (the other test's
    /// assertion fires).
    #[test]
    fn classify_attach_result_einval_yields_propagate() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "netlink_set_xdp_fd",
            io_error: io::Error::from_raw_os_error(libc::EINVAL),
        });
        let outcome: super::AttachOutcome<()> = super::classify_attach_result(Err(err));
        match outcome {
            super::AttachOutcome::Propagate(_) => {}
            other => panic!("expected AttachOutcome::Propagate(_), got {other:?}"),
        }
    }

    // ----- EBUSY → IfaceXdpSlotBusy classification (ADR-0061 § 5 / D3) -----
    //
    // The kernel permits exactly one XDP program per netdev XDP hook.
    // When the single-node default points both the forward
    // (`client_iface`) and reverse (`backend_iface`) attaches at one
    // netdev, the second attach returns `EBUSY`. The pre-fix loader
    // wrapped that in `DataplaneError::LoadFailed(format!("...DRV_MODE:
    // {e}"))`, masking the real cause behind a misleading "native
    // attach failed" string. `classify_iface_xdp_slot_busy` is the pure
    // classifier that recovers the EBUSY case BEFORE the masking
    // fallthrough — same `raw_os_error()` reach as
    // `should_fallback_to_generic`, swapping EOPNOTSUPP for EBUSY.
    //
    // Tier 3 (01-04) exercises a real EBUSY attach; these default-lane
    // unit tests drive the classifier against synthetic
    // `ProgramError::SyscallError` values — no real attach.

    /// Acceptance-shaped: an `EBUSY` attach failure on `client_iface`
    /// surfaces as [`DataplaneError::IfaceXdpSlotBusy`] carrying that
    /// iface — NOT a masking `LoadFailed`/`DRV_MODE` string. This is
    /// the scenario the variant fixes: the operator gets the honest
    /// cause (the XDP slot is already taken) and the remediation,
    /// instead of a misleading "native attach failed".
    #[test]
    fn ebusy_attach_surfaces_as_iface_xdp_slot_busy_not_masking_load_failed() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_link_create",
            io_error: io::Error::from_raw_os_error(libc::EBUSY),
        });
        let classified = super::classify_iface_xdp_slot_busy(&err, "client0");
        match classified {
            Some(DataplaneError::IfaceXdpSlotBusy { iface }) => {
                assert_eq!(iface, "client0");
            }
            other => {
                panic!("expected Some(IfaceXdpSlotBusy {{ iface: \"client0\" }}), got {other:?}")
            }
        }
    }

    /// `EPERM` is a permissions failure, not a slot collision — it must
    /// NOT classify as `IfaceXdpSlotBusy`. The classifier returns
    /// `None`, so the caller falls through to the existing
    /// `LoadFailed`/`DRV_MODE` path (which `EPERM` legitimately wants).
    #[test]
    fn eperm_attach_does_not_classify_as_iface_xdp_slot_busy() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_link_create",
            io_error: io::Error::from_raw_os_error(libc::EPERM),
        });
        assert!(super::classify_iface_xdp_slot_busy(&err, "client0").is_none());
    }

    /// `ENODEV` (interface gone) is likewise not a slot collision and
    /// must fall through. Pairs with the EBUSY test to kill the errno
    /// guard mutant: flipping `code == EBUSY` to match ENODEV would
    /// classify this case, firing the assertion.
    #[test]
    fn enodev_attach_does_not_classify_as_iface_xdp_slot_busy() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "netlink_set_xdp_fd",
            io_error: io::Error::from_raw_os_error(libc::ENODEV),
        });
        assert!(super::classify_iface_xdp_slot_busy(&err, "backend0").is_none());
    }

    /// The `IfaceXdpSlotBusy` Display contract (ADR-0061 § 5): names
    /// the iface, names the real cause (`EBUSY`), and carries the
    /// `client_iface != backend_iface` remediation hint so the
    /// operator knows the single-node default expects a dedicated
    /// veth pair.
    #[test]
    fn iface_xdp_slot_busy_display_names_iface_ebusy_and_remediation() {
        let rendered =
            DataplaneError::IfaceXdpSlotBusy { iface: "veth-client".to_owned() }.to_string();
        assert!(rendered.contains("veth-client"), "Display must name the iface: {rendered}");
        assert!(rendered.contains("EBUSY"), "Display must name the real cause EBUSY: {rendered}");
        assert!(
            rendered.contains("client_iface != backend_iface"),
            "Display must carry the client_iface != backend_iface remediation hint: {rendered}"
        );
    }

    // ----- load() vs attach() error routing for the unconnected-UDP
    //       cgroup hooks (ADR-0053 D5b/c, GH #200) -----
    //
    // A `load()` failure on `cgroup_sendmsg4_service` / `cgroup_recvmsg4_service`
    // is a BPF program-correctness problem (verifier regression, missing
    // program, capability gap) — NOT the attach-time below-floor kernel
    // refusal `CgroupSendRecvAttach` is reserved for. Routing a `load()`
    // failure through `cgroup_attach_error` would tell the operator "the
    // attach() syscall is the kernel-version oracle" for a problem that has
    // nothing to do with the kernel floor.
    //
    // There is no Tier-2 backstop (`BPF_PROG_TEST_RUN` returns ENOTSUPP for
    // `cgroup_sock_addr` programs) and a real `load()` failure cannot be
    // forced on the 5.10+ Lima matrix, so these default-lane unit tests on
    // the two error-construction helpers ARE the structural defense against
    // the call-site routing regressing. Each pins the variant its helper
    // selects; together they lock both helpers' contracts so a future swap
    // of `load()` back onto `cgroup_attach_error` flips an assertion here.

    /// `cgroup_load_error` builds a [`DataplaneError::LoadFailed`] — the
    /// program-correctness variant — NOT the kernel-floor-reserved
    /// `CgroupSendRecvAttach`. The message names the hook and `load` so all
    /// three cgroup hooks (`connect4`, `sendmsg4`, `recvmsg4`) read
    /// identically in logs.
    #[test]
    fn cgroup_load_error_yields_load_failed_not_attach_refusal() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_prog_load",
            io_error: io::Error::from_raw_os_error(libc::E2BIG),
        });
        let built = super::cgroup_load_error("cgroup_sendmsg4_service", &err);

        assert!(
            !matches!(built, DataplaneError::CgroupSendRecvAttach { .. }),
            "a load() failure must NOT route through the attach-time \
             below-floor variant: {built:?}"
        );
        match built {
            DataplaneError::LoadFailed(message) => {
                assert!(
                    message.contains("cgroup_sendmsg4_service"),
                    "LoadFailed message must name the hook: {message}"
                );
                assert!(
                    message.contains("load"),
                    "LoadFailed message must name the load() phase: {message}"
                );
            }
            other => panic!("expected DataplaneError::LoadFailed(_), got {other:?}"),
        }
    }

    /// `cgroup_attach_error` (the attach path, unchanged) builds the typed
    /// [`DataplaneError::CgroupSendRecvAttach`] carrying the offending
    /// hook. Pinned alongside the `load()` test so the two helpers'
    /// contracts are both locked — `attach()` genuinely IS the below-floor
    /// oracle per ADR-0053 D5b/c.
    #[test]
    fn cgroup_attach_error_yields_send_recv_attach_with_hook() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_prog_attach",
            io_error: io::Error::from_raw_os_error(libc::EOPNOTSUPP),
        });
        let built = super::cgroup_attach_error("cgroup_recvmsg4_service", &err);

        match built {
            DataplaneError::CgroupSendRecvAttach { hook, .. } => {
                assert_eq!(hook, "cgroup_recvmsg4_service");
            }
            other => {
                panic!("expected DataplaneError::CgroupSendRecvAttach {{ .. }}, got {other:?}")
            }
        }
    }
}
