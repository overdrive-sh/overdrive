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

// Crate-private seam over `BACKEND_MAP` (production: the real aya map).
mod backend_map;

// Direct `bpf(2)` syscall surface used where aya 0.13.x ships no
// typed wrappers (HASH_OF_MAPS construction + `BPF_PROG_TEST_RUN`).
pub mod sys;

// transparent-mtls-host-socket (ADR-0069, GH #26; OQ-2 home). The
// `HostMtlsEnforcement` adapter — the production agent-light L4 proxy over
// `cgroup_connect4` / `nft`-TPROXY intercept, rustls TLS 1.3, kTLS arm, and two
// agent-light zero-copy SPLICE pumps across the kTLS boundary: the
// forward/response path is a BLOCKING `splice` into a kTLS-TX leg (the kernel
// `tls_sw_sendmsg` encrypts each spliced chunk inside the blocking call —
// `findings-ktls-tx-blocking-splice.md`), the return/deliver path is a zero-copy
// `splice` out of a kTLS-RX leg (`tls_sw_splice_read` decrypts on splice-out).
// Consumes `IdentityRead` (#35); kTLS arms on the agent's leg, never the
// workload's socket.
pub mod mtls;

// public-ingress-gateway (G1, ADR-0244/0246) — `HostGatewayRelay`, the host backend of
// the `overdrive_gateway::ports::GatewayRelay` driven port: kernel-TLS arming
// after a userspace rustls handshake, control-record dispatch, private-pipe
// `splice`, and the Earned-Trust probe. Its `sys` module is the only one with
// `unsafe`.
pub mod gateway_relay;

// public-ingress-gateway (ADR-0228/0232/0233/0243) — `GatewayConnectDataplane`
// for `EbpfDataplane`: upstream-socket preparation, the connect-intent and
// selection-receipt map ABI, applied backend identity.
pub mod gateway_connect;

// QC-3: the quint-connect conformance driver for `selection_receipt`
// (`specs/quint/public-ingress-gateway/selection_receipt.qnt`, main
// `receipt_conf`) against the real `EbpfDataplane`.
#[cfg(all(test, feature = "integration-tests"))]
mod quint_selection_receipt;

// S-PIG-132: the fault decorator over the `BACKEND_MAP` seam and the
// rollback-restore scenario that uses it.
#[cfg(all(test, feature = "integration-tests"))]
mod service_apply_rollback_faults;

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

/// Default cgroup attach path for `cgroup_connect4_gateway`.
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

/// Number of Maglev slots in each `SERVICE_MAP` inner array: the Maglev table
/// size, [`overdrive_core::dataplane::MaglevTableSize::DEFAULT`].get() =
/// 16_381 (architecture.md § 5 Q-Sig D6 / ADR-0041). This is the modulus the
/// XDP program reduces the 5-tuple hash by, and it **MUST** stay in lockstep
/// with `INNER_TABLE_SIZE` in `crates/overdrive-bpf/src/maps/service_map.rs`
/// (kernel-side). The XDP program never selects a slot at or beyond it.
const SERVICE_MAP_TABLE_SIZE: u32 = overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();

/// Inner slot holding the low 32 bits of the applied-set fingerprint (D-20).
const FINGERPRINT_LOW_SLOT: u32 = SERVICE_MAP_TABLE_SIZE;

/// Inner slot holding the high 32 bits of the applied-set fingerprint (D-20).
const FINGERPRINT_HIGH_SLOT: u32 = SERVICE_MAP_TABLE_SIZE + 1;

/// `SERVICE_MAP` inner-ARRAY capacity in slots: the Maglev table plus the two
/// fingerprint slots, 16_383. The inner prototype and every inner map carry
/// exactly this many slots, because the kernel rejects an inner map whose
/// `max_entries` differs from the prototype's (K-4: `bpf_map_meta_equal`,
/// `kernel/bpf/map_in_map.c:83-92 @ v7.0`; `array_map_meta_equal`,
/// `kernel/bpf/arraymap.c:594-601 @ v7.0`). This is distinct from the
/// 16_381 Maglev modulus above; the two must never be conflated.
const SERVICE_MAP_INNER_CAPACITY: u32 = SERVICE_MAP_TABLE_SIZE + 2;

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

    /// Userspace handle to BACKEND_MAP (regular HASH). Owns the
    /// read/write surface for resolved backend records.
    ///
    /// Every access goes through the crate-private `BackendMapOps`
    /// seam; the production implementation is the aya hash map.
    ///
    /// Wrapped in `parking_lot::Mutex` because the seam's mutating
    /// operations (insert / remove, mirroring aya's `&mut self`
    /// `HashMap` methods) need exclusive access, but
    /// `Dataplane::update_service` is `&self` (the trait surface is
    /// the canonical interior-mutability boundary for BPF map
    /// updates). The lock is held only for the duration of the BPF
    /// syscalls — never across `.await`.
    backend_map: parking_lot::Mutex<Box<dyn crate::backend_map::BackendMapOps + Send>>,

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
    /// purge invariant). A key has exactly one owning Service
    /// (R-59 rejects an update naming a key another VIP owns), so
    /// the keys that left this Service's set are exactly the keys
    /// to purge.
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

    /// Userspace handle to `GATEWAY_CONNECT_INTENT_MAP` (public-ingress-gateway,
    /// ADR-0232 / ADR-0243): one registered intent per prepared gateway upstream
    /// socket, keyed by the socket cookie. Unpinned: it lives with the loaded BPF
    /// object. `parking_lot::Mutex` because aya's `HashMap` writes take `&mut
    /// self`; never held across `.await`.
    gateway_intent_map: parking_lot::Mutex<
        aya::maps::HashMap<
            aya::maps::MapData,
            u64,
            crate::gateway_connect::GatewayConnectIntentPod,
        >,
    >,

    /// Userspace handle to `GATEWAY_SELECTION_RECEIPT_MAP`: the receipt the
    /// gateway selection arm writes before it allows or denies a registered
    /// connect, keyed by the same cookie.
    gateway_receipt_map: parking_lot::Mutex<
        aya::maps::HashMap<
            aya::maps::MapData,
            u64,
            crate::gateway_connect::GatewaySelectionReceiptPod,
        >,
    >,

    /// Applied backend identity of every `BackendId` the Service maps name.
    /// Read by `GatewayConnectDataplane::selected_backend_identity`.
    backend_identities: parking_lot::Mutex<
        std::collections::BTreeMap<u32, crate::gateway_connect::BackendIdentityState>,
    >,

    /// The commit guard of `update_service` (ADR-0233): writers serialise on it
    /// and `selected_backend_identity` readers wait for it, so an identity is
    /// readable only after the outer `SERVICE_MAP` swap and its `Applied`
    /// publication.
    service_apply_commit: parking_lot::RwLock<()>,

    /// Reverse-NAT keys a post-commit cleanup or a partial rollback could not
    /// delete; retried by the next `update_service`.
    service_cleanup_debt:
        parking_lot::Mutex<std::collections::BTreeSet<crate::maps::BackendKeyPod>>,

    /// Owns the `cgroup_connect4_gateway` attachment (R-1): the gateway
    /// selection program. Dropping detaches the program (RAII, the same shape as
    /// the XDP links above: aya's `CgroupSockAddrLinkId` closes the
    /// `bpf_link_create` fd, which detaches at the kernel-link refcount boundary).
    #[allow(dead_code)]
    _cgroup_connect4_gateway_link: aya::programs::cgroup_sock_addr::CgroupSockAddrLinkId,

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
        use crate::maps::{BackendEntryPod, BackendKeyPod, ServiceKey, VipPod};

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
        if let Err(e) = std::fs::remove_file(&pin_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(DataplaneError::LoadFailed(format!(
                "unlink stale pin {}: {e}",
                pin_path.display()
            )));
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

        // Open the attach cgroup directory FD (read-only is sufficient; aya's
        // `attach` passes it through to `bpf_link_create(LinkTarget::Fd(cgroup_fd))`).
        // The operator-supplied cgroup_attach_path MUST exist.
        let cgroup_file = std::fs::File::open(cgroup_attach_path).map_err(|e| {
            DataplaneError::LoadFailed(format!(
                "open cgroup_attach_path {}: {e}",
                cgroup_attach_path.display()
            ))
        })?;

        // public-ingress-gateway (ADR-0232 / ADR-0243) — recover the two
        // unpinned gateway maps (aya creates them at load) and load + attach the
        // gateway selection program.
        let gateway_intent_map = aya::maps::HashMap::<
            _,
            u64,
            crate::gateway_connect::GatewayConnectIntentPod,
        >::try_from(
            bpf.take_map("GATEWAY_CONNECT_INTENT_MAP").ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "GATEWAY_CONNECT_INTENT_MAP not found in BPF object".into(),
                )
            })?,
        )
        .map_err(|e| {
            DataplaneError::LoadFailed(format!("GATEWAY_CONNECT_INTENT_MAP try_from: {e}"))
        })?;
        let gateway_receipt_map = aya::maps::HashMap::<
            _,
            u64,
            crate::gateway_connect::GatewaySelectionReceiptPod,
        >::try_from(
            bpf.take_map("GATEWAY_SELECTION_RECEIPT_MAP").ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "GATEWAY_SELECTION_RECEIPT_MAP not found in BPF object".into(),
                )
            })?,
        )
        .map_err(|e| {
            DataplaneError::LoadFailed(format!("GATEWAY_SELECTION_RECEIPT_MAP try_from: {e}"))
        })?;

        let gateway_prog: &mut CgroupSockAddr = bpf
            .program_mut("cgroup_connect4_gateway")
            .ok_or_else(|| {
                DataplaneError::LoadFailed(
                    "cgroup_connect4_gateway program not found in BPF object".into(),
                )
            })?
            .try_into()
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("cgroup_connect4_gateway program type: {e}"))
            })?;
        gateway_prog.load().map_err(|e| cgroup_load_error("cgroup_connect4_gateway", &e))?;
        // Attach mode: aya's `CgroupAttachMode::Single` names the link flags it
        // passes (none); the kernel lists a link-based attach as `multi`. A link
        // attach is always multi: `cgroup_bpf_link_attach` passes
        // `BPF_F_ALLOW_MULTI | attr->link_create.flags`
        // (`kernel/bpf/cgroup.c:1506-1507 @ v7.0`). A deny yields `-EPERM`
        // (`kernel/bpf/cgroup.c:79-89 @ v7.0`).
        let cgroup_connect4_gateway_link_id =
            gateway_prog.attach(&cgroup_file, CgroupAttachMode::Single).map_err(|e| {
                DataplaneError::LoadFailed(format!(
                    "cgroup_connect4_gateway.attach({}): {e}",
                    cgroup_attach_path.display()
                ))
            })?;

        Ok(Self {
            bpf,
            service_map,
            backend_map: parking_lot::Mutex::new(Box::new(backend_map)),
            reverse_nat_map: parking_lot::Mutex::new(reverse_nat_map),
            backend_id_alloc: parking_lot::Mutex::new(crate::allocators::BackendIdAllocator::new()),
            service_backends: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            service_reverse_nat_keys: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            pin_dir: pin_dir.to_path_buf(),
            _xdp_forward_link: xdp_forward_link,
            _xdp_reverse_link: xdp_reverse_link,
            gateway_intent_map: parking_lot::Mutex::new(gateway_intent_map),
            gateway_receipt_map: parking_lot::Mutex::new(gateway_receipt_map),
            backend_identities: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            service_apply_commit: parking_lot::RwLock::new(()),
            service_cleanup_debt: parking_lot::Mutex::new(std::collections::BTreeSet::new()),
            _cgroup_connect4_gateway_link: cgroup_connect4_gateway_link_id,
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
                .get(key)
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
            .insert(SENTINEL_BACKEND_ID, sentinel)
            .map_err(|e| DataplaneError::LoadFailed(format!("probe: BACKEND_MAP insert: {e}")))?;

        // Step 2 — read-back. Bind the lookup `Result` to a local so
        // the `parking_lot::MutexGuard` drops before the `match`
        // arm runs — keeps clippy's
        // `significant_drop_in_scrutinee` lint satisfied without
        // holding the lock across the error-formatting path.
        let read_back = self.backend_map.lock().get(SENTINEL_BACKEND_ID);
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
            let _ = self.backend_map.lock().remove(SENTINEL_BACKEND_ID);
            return Err(DataplaneError::LoadFailed(format!(
                "probe: round-trip mismatch (got {got:?}, want {sentinel:?})"
            )));
        }

        // Step 4 — delete. After a clean delete, `BACKEND_MAP` is
        // byte-equal to its pre-probe state.
        self.backend_map
            .lock()
            .remove(SENTINEL_BACKEND_ID)
            .map_err(|e| DataplaneError::LoadFailed(format!("probe: BACKEND_MAP delete: {e}")))?;

        Ok(())
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
        if let Err(e) = std::fs::remove_file(&pin_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(
                name: "xdp.shutdown.unlink_failed",
                path = %pin_path.display(),
                error = %e,
                "SERVICE_MAP pin unlink failed during shutdown"
            );
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

/// Build the typed [`DataplaneError::LoadFailed`] for a failed `load()` of a
/// cgroup hook. A load failure is a BPF program-correctness problem (verifier
/// rejection, missing program, capability gap).
fn cgroup_load_error(hook: &'static str, error: &aya::programs::ProgramError) -> DataplaneError {
    DataplaneError::LoadFailed(format!("{hook}.load: {error}"))
}

/// One backend of a planned `update_service` generation: the id it will carry,
/// its `BACKEND_MAP` value and the allocation whose identity the id denotes.
struct PlannedBackend {
    id: overdrive_core::id::BackendId,
    pod: crate::maps::BackendEntryPod,
    alloc: overdrive_core::id::SpiffeId,
    /// The id was minted by this update (no allocator memo entry before it).
    fresh: bool,
}

/// What a pre-commit failure must restore (ADR-0233): only keys proven absent
/// before this update are deleted, and an in-place `BACKEND_MAP` update restores
/// the value it replaced.
#[derive(Default)]
struct ServiceApplyUndo {
    inserted_backend_ids: Vec<overdrive_core::id::BackendId>,
    updated_backend_pods: Vec<(overdrive_core::id::BackendId, crate::maps::BackendEntryPod)>,
    inserted_reverse_keys: Vec<crate::maps::BackendKeyPod>,
}

/// Convert a `HashOfMaps` failure into the dataplane error; `context` names the
/// operation in the `LoadFailed` text.
fn hash_of_maps_error(
    context: &str,
    error: crate::maps::hash_of_maps::HashOfMapsError,
) -> DataplaneError {
    match error {
        crate::maps::hash_of_maps::HashOfMapsError::MapAllocFailed { source } => {
            DataplaneError::MapAllocFailed { source }
        }
        crate::maps::hash_of_maps::HashOfMapsError::Syscall(source) => {
            DataplaneError::LoadFailed(format!("{context}: {source}"))
        }
    }
}

/// A `ServiceVip` from a stored host-order IPv4 address.
fn service_vip(ip_host: u32) -> Result<overdrive_core::id::ServiceVip, DataplaneError> {
    overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(Ipv4Addr::from(ip_host)))
        .map_err(|e| DataplaneError::LoadFailed(format!("service vip {ip_host}: {e}")))
}

/// The commit-guarded Service-map and applied-identity transaction
/// (ADR-0228, ADR-0232, ADR-0233, ADR-0241). Every method here runs under the
/// `service_apply_commit` write guard `update_service` holds.
impl EbpfDataplane {
    /// Validate the input, reject an identity conflict before any mutation, then
    /// mint ids for the endpoints the allocator has not seen and reserve their
    /// identities (not yet readable).
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    fn plan_service_apply(
        &self,
        service_key: crate::maps::wire::ServiceKey,
        svc_proto: overdrive_core::dataplane::Proto,
        backends: &[Backend],
    ) -> Result<Vec<PlannedBackend>, DataplaneError> {
        use crate::gateway_connect::BackendIdentityState;
        use overdrive_core::dataplane::backend_key::BackendKey;

        use crate::maps::{BackendEntryPod, BackendKeyPod};

        let proto = svc_proto.as_u8();

        let mut by_endpoint: std::collections::BTreeMap<
            (u32, u16),
            (BackendEntryPod, &overdrive_core::id::SpiffeId),
        > = std::collections::BTreeMap::new();
        for backend in backends {
            let pod = BackendEntryPod::from_backend(backend)?;
            let endpoint = (pod.ipv4_host, pod.port_host);
            if let Some((_, held)) = by_endpoint.get(&endpoint)
                && **held != backend.workload_identity
            {
                return Err(DataplaneError::LoadFailed(format!(
                    "backend endpoint {} carries two allocations: {held} and {}",
                    backend.addr, backend.workload_identity
                )));
            }
            by_endpoint.insert(endpoint, (pod, &backend.workload_identity));
        }

        // Planning order: the identity conflict is reported first, then R-59.
        {
            let allocator = self.backend_id_alloc.lock();
            let identities = self.backend_identities.lock();
            for ((ip_host, port_host), (_, attempted)) in &by_endpoint {
                let Some(backend_id) = allocator.existing_id(*ip_host, *port_host, proto) else {
                    continue;
                };
                let Some(state) = identities.get(&backend_id.get()) else {
                    continue;
                };
                if state.spiffe_id() != *attempted {
                    return Err(DataplaneError::BackendIdentityConflict {
                        backend_id,
                        current: state.spiffe_id().clone(),
                        attempted: (*attempted).clone(),
                    });
                }
            }
        }

        // R-59: a reverse-NAT key another Service's VIP already owns is
        // rejected before anything is reserved or written.
        {
            let reverse_nat_map = self.reverse_nat_map.lock();
            for (ip_host, port_host) in by_endpoint.keys() {
                let key =
                    BackendKeyPod { ip_host: *ip_host, port_host: *port_host, proto, _pad: 0 };
                match reverse_nat_map.get(&key, 0) {
                    Ok(owner) if owner.ip_host != service_key.vip_host => {
                        return Err(DataplaneError::ReverseNatConflict {
                            key: BackendKey::new(Ipv4Addr::from(*ip_host), *port_host, svc_proto),
                            current_vip: service_vip(owner.ip_host)?,
                            attempted_vip: service_vip(service_key.vip_host)?,
                        });
                    }
                    Ok(_) | Err(aya::maps::MapError::KeyNotFound) => {}
                    Err(e) => {
                        return Err(DataplaneError::LoadFailed(format!(
                            "REVERSE_NAT_MAP get: {e}"
                        )));
                    }
                }
            }
        }

        let mut allocator = self.backend_id_alloc.lock();
        let mut identities = self.backend_identities.lock();

        let mut planned = Vec::with_capacity(by_endpoint.len());
        for ((ip_host, port_host), (pod, alloc)) in by_endpoint {
            let fresh = allocator.existing_id(ip_host, port_host, proto).is_none();
            let id = allocator.allocate(ip_host, port_host, proto);
            if fresh {
                identities
                    .insert(id.get(), BackendIdentityState::Reserved { spiffe_id: alloc.clone() });
            }
            planned.push(PlannedBackend { id, pod, alloc: alloc.clone(), fresh });
        }
        Ok(planned)
    }

    /// Populate a fresh inner array: the Maglev permutation (a deterministic
    /// function of the weighted backend set, S-2.2-12) in slots
    /// `0..SERVICE_MAP_TABLE_SIZE`, then the applied-set fingerprint in the two
    /// slots after it. The fingerprint rides in the NEW inner array, so the one
    /// atomic outer swap publishes it with the table it describes and it
    /// disappears with it; the Maglev loop never touches those slots.
    fn populate_inner_map(
        new_inner: std::os::fd::BorrowedFd<'_>,
        planned: &[PlannedBackend],
        fingerprint: overdrive_core::dataplane::fingerprint::BackendSetFingerprint,
    ) -> Result<(), DataplaneError> {
        let weighted: std::collections::BTreeMap<overdrive_core::id::BackendId, u16> =
            planned.iter().map(|backend| (backend.id, backend.pod.weight.max(1))).collect();
        let permutation = overdrive_core::maglev::permutation::generate(
            &weighted,
            overdrive_core::dataplane::MaglevTableSize::DEFAULT,
        );
        if permutation.len() != SERVICE_MAP_TABLE_SIZE as usize {
            return Err(DataplaneError::LoadFailed(format!(
                "maglev::generate returned {} slots; expected {}",
                permutation.len(),
                SERVICE_MAP_TABLE_SIZE
            )));
        }
        for (slot, bid) in permutation.iter().enumerate() {
            // Slot is bounded by the permutation length check above
            // (SERVICE_MAP_TABLE_SIZE = 16_381, well within u32).
            #[allow(clippy::cast_possible_truncation)]
            let key_bytes = (slot as u32).to_ne_bytes();
            crate::sys::bpf::bpf_map_update_elem(
                new_inner,
                &key_bytes,
                &bid.get().to_ne_bytes(),
                crate::sys::bpf::BPF_ANY,
            )
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("inner-map slot {slot} populate: {e}"))
            })?;
        }
        for (slot, word) in [
            (FINGERPRINT_LOW_SLOT, fingerprint_low_word(fingerprint)),
            (FINGERPRINT_HIGH_SLOT, fingerprint_high_word(fingerprint)),
        ] {
            crate::sys::bpf::bpf_map_update_elem(
                new_inner,
                &slot.to_ne_bytes(),
                &word.to_ne_bytes(),
                crate::sys::bpf::BPF_ANY,
            )
            .map_err(|e| {
                DataplaneError::LoadFailed(format!("inner-map fingerprint slot {slot}: {e}"))
            })?;
        }
        Ok(())
    }

    /// Everything up to and including the one atomic outer `SERVICE_MAP` swap.
    /// A failure names the stage it happened at; `undo` records what to restore.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    fn precommit_service_apply(
        &self,
        planned: &[PlannedBackend],
        service_key: crate::maps::wire::ServiceKey,
        new_keys: &std::collections::BTreeSet<crate::maps::BackendKeyPod>,
        fingerprint: overdrive_core::dataplane::fingerprint::BackendSetFingerprint,
        undo: &mut ServiceApplyUndo,
    ) -> Result<(), (overdrive_core::traits::dataplane::ServiceApplyStage, DataplaneError)> {
        use std::os::fd::AsFd;

        use overdrive_core::traits::dataplane::ServiceApplyStage as Stage;

        // Stage BackendMap — an identical entry is reused untouched, a differing
        // `weight`/`healthy` is the ordinary in-place update (prior value saved),
        // an absent key is inserted.
        {
            let mut backend_map = self.backend_map.lock();
            for backend in planned {
                let key = backend.id.get();
                let prior = match backend_map.get(key) {
                    Ok(prior) if prior == backend.pod => continue,
                    Ok(prior) => Some(prior),
                    Err(aya::maps::MapError::KeyNotFound) => None,
                    Err(e) => {
                        return Err((
                            Stage::BackendMap,
                            DataplaneError::LoadFailed(format!("BACKEND_MAP get: {e}")),
                        ));
                    }
                };
                backend_map.insert(key, backend.pod).map_err(|e| {
                    (
                        Stage::BackendMap,
                        DataplaneError::LoadFailed(format!("BACKEND_MAP insert: {e}")),
                    )
                })?;
                match prior {
                    Some(prior) => undo.updated_backend_pods.push((backend.id, prior)),
                    None => undo.inserted_backend_ids.push(backend.id),
                }
            }
        }

        // Stage InnerMap — a fresh private inner ARRAY populated with the Maglev
        // permutation (a deterministic function of the weighted backend set,
        // S-2.2-12). The current outer pointer is untouched, so no new id is
        // selectable yet.
        let new_inner = self
            .service_map
            .create_inner(None)
            .map_err(|e| (Stage::InnerMap, hash_of_maps_error("inner-map alloc", e)))?;
        Self::populate_inner_map(new_inner.as_fd(), planned, fingerprint)
            .map_err(|e| (Stage::InnerMap, e))?;

        // Stage ReverseNat — MUST land before the outer swap so a response from a
        // newly-added backend never misses the reverse map and escapes with the
        // backend IP as source. Only absent keys are recorded for rollback.
        let vip_value = crate::maps::VipPod {
            ip_host: service_key.vip_host,
            port_host: service_key.port_host,
            _pad: 0,
        };
        {
            let mut reverse_nat_map = self.reverse_nat_map.lock();
            for key in new_keys {
                let present = match reverse_nat_map.get(key, 0) {
                    Ok(_) => true,
                    Err(aya::maps::MapError::KeyNotFound) => false,
                    Err(e) => {
                        return Err((
                            Stage::ReverseNat,
                            DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP get: {e}")),
                        ));
                    }
                };
                reverse_nat_map.insert(key, vip_value, 0).map_err(|e| {
                    (
                        Stage::ReverseNat,
                        DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP insert: {e}")),
                    )
                })?;
                if !present {
                    undo.inserted_reverse_keys.push(*key);
                }
            }
        }

        // Stage OuterCommit — the single atomic outer-pointer update. The kernel
        // ref-counts the new and old inner maps; concurrent XDP readers see one or
        // the other. THIS IS THE LOAD-BEARING STEP and the commit point.
        self.service_map
            .set(&service_key, new_inner.as_fd())
            .map_err(|e| (Stage::OuterCommit, hash_of_maps_error("SERVICE_MAP outer set", e)))
    }

    /// Replay `undo` after a pre-commit failure: delete only keys proven absent
    /// before this update (every one is attempted even after a failure), restore
    /// in-place updates, remove this update's reserved identities and release its
    /// fresh allocator memos (their numeric ids are never recycled). The prior
    /// outer pointer was never touched. Returns the original error, or
    /// `ServiceApplyRollback` when a restore (an in-place `BACKEND_MAP` restore
    /// or a delete) also failed (R-60).
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    fn rollback_service_apply(
        &self,
        stage: overdrive_core::traits::dataplane::ServiceApplyStage,
        error: DataplaneError,
        undo: &ServiceApplyUndo,
        planned: &[PlannedBackend],
        proto: overdrive_core::dataplane::Proto,
    ) -> DataplaneError {
        use overdrive_core::traits::dataplane::ServiceApplyRollbackFailure as Failure;

        let mut failures = Vec::new();
        {
            let mut backend_map = self.backend_map.lock();
            for (id, prior) in undo.updated_backend_pods.iter().rev() {
                if backend_map.insert(id.get(), *prior).is_err() {
                    failures.push(Failure::BackendRestore(*id));
                }
            }
            for id in undo.inserted_backend_ids.iter().rev() {
                match backend_map.remove(id.get()) {
                    Ok(()) => {}
                    Err(e) if crate::gateway_connect::is_key_absent(&e) => {}
                    Err(_) => failures.push(Failure::BackendDelete(*id)),
                }
            }
        }
        {
            let mut reverse_nat_map = self.reverse_nat_map.lock();
            let mut debt = self.service_cleanup_debt.lock();
            for key in undo.inserted_reverse_keys.iter().rev() {
                match reverse_nat_map.remove(key) {
                    Ok(()) => {}
                    Err(e) if crate::gateway_connect::is_key_absent(&e) => {}
                    Err(_) => {
                        debt.insert(*key);
                        failures.push(Failure::ReverseNatDelete {
                            backend: std::net::SocketAddrV4::new(
                                Ipv4Addr::from(key.ip_host),
                                key.port_host,
                            ),
                            proto,
                        });
                    }
                }
            }
        }
        {
            let mut allocator = self.backend_id_alloc.lock();
            let mut identities = self.backend_identities.lock();
            for backend in planned.iter().filter(|backend| backend.fresh) {
                identities.remove(&backend.id.get());
                allocator.release(backend.id);
            }
        }
        if failures.is_empty() {
            error
        } else {
            DataplaneError::ServiceApplyRollback { stage, rollback: failures }
        }
    }

    /// Non-empty update: plan, pre-commit, commit, then publish every identity
    /// `Applied` before the guard releases.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    fn apply_service(
        &self,
        service_key: crate::maps::wire::ServiceKey,
        svc_proto: overdrive_core::dataplane::Proto,
        backends: &[Backend],
        fingerprint: overdrive_core::dataplane::fingerprint::BackendSetFingerprint,
    ) -> Result<(), DataplaneError> {
        use crate::gateway_connect::BackendIdentityState;
        use crate::maps::BackendKeyPod;

        let proto = svc_proto.as_u8();
        let planned = self.plan_service_apply(service_key, svc_proto, backends)?;
        let new_keys: std::collections::BTreeSet<BackendKeyPod> = planned
            .iter()
            .map(|backend| BackendKeyPod {
                ip_host: backend.pod.ipv4_host,
                port_host: backend.pod.port_host,
                proto,
                _pad: 0,
            })
            .collect();

        let mut undo = ServiceApplyUndo::default();
        if let Err((stage, error)) =
            self.precommit_service_apply(&planned, service_key, &new_keys, fingerprint, &mut undo)
        {
            return Err(self.rollback_service_apply(stage, error, &undo, &planned, svc_proto));
        }

        // Committed. Publishing is infallible in-memory mutation: a receipt that
        // names one of these ids can resolve its identity only after this point,
        // because its read waits for the commit guard.
        {
            let mut identities = self.backend_identities.lock();
            for backend in &planned {
                identities.insert(
                    backend.id.get(),
                    BackendIdentityState::Applied { spiffe_id: backend.alloc.clone() },
                );
            }
        }
        self.service_backends
            .lock()
            .insert(service_key, planned.iter().map(|backend| backend.id.get()).collect());
        let stale_reverse: std::collections::BTreeSet<BackendKeyPod> = {
            let mut tracker = self.service_reverse_nat_keys.lock();
            let prior = tracker.get(&service_key).cloned().unwrap_or_default();
            tracker.insert(service_key, new_keys.clone());
            prior.difference(&new_keys).copied().collect()
        };
        self.settle_cleanup(stale_reverse);
        Ok(())
    }

    /// Empty update: the outer-slot delete is the commit; historical Applied
    /// identities stay readable for old receipts; the idempotent cleanup follows.
    fn withdraw_service(
        &self,
        frontend_key: crate::maps::wire::ServiceKey,
    ) -> Result<(), DataplaneError> {
        let matching_keys: Vec<crate::maps::wire::ServiceKey> = {
            let tracker = self.service_backends.lock();
            empty_backend_purge_keys(tracker.keys(), frontend_key)
        };
        if matching_keys.is_empty() {
            return Ok(());
        }
        for service_key in &matching_keys {
            self.service_map
                .delete(service_key)
                .map_err(|e| hash_of_maps_error("SERVICE_MAP outer delete", e))?;
        }
        {
            let mut tracker = self.service_backends.lock();
            for service_key in &matching_keys {
                tracker.remove(service_key);
            }
        }
        let stale_reverse = {
            let mut tracker = self.service_reverse_nat_keys.lock();
            matching_keys.iter().flat_map(|key| tracker.remove(key).unwrap_or_default()).collect()
        };
        self.settle_cleanup(stale_reverse);
        Ok(())
    }

    /// Post-commit orphan cleanup (ADR-0233 step 6). Never turns a committed
    /// update into a failure: a `BACKEND_MAP` orphan that cannot be removed is
    /// swept again by the next update's full scan, and a reverse-NAT key that
    /// cannot be removed stays in `service_cleanup_debt` until a later update
    /// removes it. Applied identities are never removed.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    fn settle_cleanup(
        &self,
        stale_reverse: std::collections::BTreeSet<crate::maps::BackendKeyPod>,
    ) {
        let live_ids: std::collections::BTreeSet<u32> =
            self.service_backends.lock().values().flat_map(|ids| ids.iter().copied()).collect();
        {
            let mut backend_map = self.backend_map.lock();
            let released = match crate::gc::sweep_orphan_backends_in(&mut **backend_map, &live_ids)
            {
                Ok(removed) => removed,
                Err(crate::gc::GcError::Remove {
                    offending_id,
                    removed_before_failure,
                    source,
                }) => {
                    tracing::warn!(
                        name: "dataplane.service_cleanup.deferred",
                        backend_id = offending_id,
                        error = %source,
                        "BACKEND_MAP orphan removal deferred to the next update"
                    );
                    removed_before_failure
                }
                Err(crate::gc::GcError::KeysIteration { source }) => {
                    tracing::warn!(
                        name: "dataplane.service_cleanup.deferred",
                        error = %source,
                        "BACKEND_MAP orphan sweep deferred to the next update"
                    );
                    Vec::new()
                }
            };
            let mut allocator = self.backend_id_alloc.lock();
            for removed_id in released {
                if let Ok(bid) = overdrive_core::id::BackendId::new(removed_id) {
                    allocator.release(bid);
                }
            }
        }

        self.settle_reverse_nat_debt(stale_reverse);
    }

    /// Delete the reverse-NAT keys in `newly_stale` and every key earlier
    /// cleanups could not delete. A key that still fails stays in
    /// `service_cleanup_debt` as closed debt; a key its owner has since
    /// re-claimed is dropped from the debt untouched. The caller holds
    /// `service_apply_commit`.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the guards are scoped to the section whose all-or-nothing outcome they protect"
    )]
    pub(crate) fn settle_reverse_nat_debt(
        &self,
        newly_stale: std::collections::BTreeSet<crate::maps::BackendKeyPod>,
    ) {
        let live_nat: std::collections::BTreeSet<crate::maps::BackendKeyPod> = self
            .service_reverse_nat_keys
            .lock()
            .values()
            .flat_map(|keys| keys.iter().copied())
            .collect();
        let mut reverse_nat_map = self.reverse_nat_map.lock();
        let mut debt = self.service_cleanup_debt.lock();
        debt.extend(newly_stale);
        debt.retain(|key| {
            // A debt key its owner has since re-claimed is live again: drop the
            // debt without deleting the entry.
            if live_nat.contains(key) {
                return false;
            }
            match reverse_nat_map.remove(key) {
                Ok(()) => false,
                Err(e) if crate::gateway_connect::is_key_absent(&e) => false,
                Err(e) => {
                    tracing::warn!(
                        name: "dataplane.service_cleanup.deferred",
                        error = %e,
                        "REVERSE_NAT_MAP key removal deferred to the next update"
                    );
                    true
                }
            }
        });
    }
}

/// Low 32 bits of a fingerprint, the value of inner slot [`FINGERPRINT_LOW_SLOT`].
fn fingerprint_low_word(
    fingerprint: overdrive_core::dataplane::fingerprint::BackendSetFingerprint,
) -> u32 {
    u32::try_from(fingerprint & u64::from(u32::MAX))
        .unwrap_or_else(|_| unreachable!("a value masked to 32 bits fits in u32"))
}

/// High 32 bits of a fingerprint, the value of inner slot [`FINGERPRINT_HIGH_SLOT`].
fn fingerprint_high_word(
    fingerprint: overdrive_core::dataplane::fingerprint::BackendSetFingerprint,
) -> u32 {
    u32::try_from(fingerprint >> 32)
        .unwrap_or_else(|_| unreachable!("a u64 shifted right by 32 fits in u32"))
}

impl EbpfDataplane {
    /// Read the fingerprint of `service_key`'s programmed inner map: outer
    /// lookup (the inner map id), `BPF_MAP_GET_FD_BY_ID`, then the two
    /// fingerprint slots, recombined `low | high << 32`. `Ok(None)` only when
    /// no outer slot is programmed; every failed kernel read is
    /// [`DataplaneError::LoadFailed`].
    ///
    /// The caller holds `service_apply_commit` for read for the whole
    /// sequence: an update touches several maps and is not atomic to a reader
    /// without the guard (`.claude/rules/bpf.md` § "Reading kernel maps from
    /// userspace"). Kernel facts (all `@ v7.0`): an outer lookup from userspace
    /// returns the inner map's id (K-1: `bpf_fd_htab_map_lookup_elem`,
    /// `kernel/bpf/hashtab.c:2626-2643`; `map_fd_sys_lookup_elem`,
    /// `kernel/bpf/map_in_map.c:131-134`); `BPF_MAP_GET_FD_BY_ID` opens it and
    /// returns `ENOENT` once the last reference to a replaced inner map drops,
    /// which is why the guard is held (K-2: `bpf_map_get_fd_by_id`,
    /// `kernel/bpf/syscall.c:4859-4888`); the outer swap is atomic for readers
    /// (K-3: `bpf_fd_htab_map_update_elem`, `kernel/bpf/hashtab.c:2646-2667`).
    fn read_programmed_fingerprint(
        &self,
        service_key: crate::maps::wire::ServiceKey,
    ) -> Result<Option<overdrive_core::dataplane::fingerprint::BackendSetFingerprint>, DataplaneError>
    {
        let _commit = self.service_apply_commit.read();

        // SAFETY: `ServiceKey` is `repr(C)` plain data with a zeroed pad byte.
        let key_bytes = unsafe {
            core::slice::from_raw_parts(
                (&raw const service_key).cast::<u8>(),
                core::mem::size_of::<crate::maps::wire::ServiceKey>(),
            )
        };
        let outer = crate::sys::bpf::bpf_map_lookup_elem(
            self.service_map.as_fd(),
            key_bytes,
            core::mem::size_of::<u32>(),
        )
        .map_err(|e| DataplaneError::LoadFailed(format!("SERVICE_MAP outer lookup: {e}")))?;
        let Some(id_bytes) = outer else {
            return Ok(None);
        };
        let inner_id = u32::from_ne_bytes(id_bytes.try_into().map_err(|_| {
            DataplaneError::LoadFailed("SERVICE_MAP outer value is not a 4-byte id".to_owned())
        })?);
        let inner = crate::sys::bpf::bpf_map_get_fd_by_id(inner_id).map_err(|e| {
            DataplaneError::LoadFailed(format!("SERVICE_MAP inner map {inner_id} open: {e}"))
        })?;
        let low = Self::read_fingerprint_slot(&inner, FINGERPRINT_LOW_SLOT)?;
        let high = Self::read_fingerprint_slot(&inner, FINGERPRINT_HIGH_SLOT)?;
        Ok(Some(u64::from(low) | (u64::from(high) << 32)))
    }

    fn read_fingerprint_slot(
        inner: &std::os::fd::OwnedFd,
        slot: u32,
    ) -> Result<u32, DataplaneError> {
        use std::os::fd::AsFd;

        let value = crate::sys::bpf::bpf_map_lookup_elem(
            inner.as_fd(),
            &slot.to_ne_bytes(),
            core::mem::size_of::<u32>(),
        )
        .map_err(|e| DataplaneError::LoadFailed(format!("inner-map slot {slot} read: {e}")))?
        .ok_or_else(|| DataplaneError::LoadFailed(format!("inner-map slot {slot} is absent")))?;
        let word: [u8; 4] = value.try_into().map_err(|_| {
            DataplaneError::LoadFailed(format!("inner-map slot {slot} is not a 4-byte value"))
        })?;
        Ok(u32::from_ne_bytes(word))
    }
}

#[async_trait]
impl overdrive_core::traits::ProgrammedServiceView for EbpfDataplane {
    /// Read the fingerprint written with the table behind `frontend`'s outer
    /// `SERVICE_MAP` slot. `Ok(None)` when no slot is programmed (never
    /// programmed, withdrawn, or a restarted dataplane over recreated maps).
    async fn programmed_fingerprint(
        &self,
        frontend: &overdrive_core::dataplane::ServiceFrontend,
    ) -> Result<Option<overdrive_core::dataplane::fingerprint::BackendSetFingerprint>, DataplaneError>
    {
        let service_key = crate::maps::wire::ServiceKey {
            vip_host: u32::from(frontend.vip_v4()),
            port_host: frontend.port().get(),
            proto: frontend.proto().as_u8(),
            _pad: 0,
        };
        self.read_programmed_fingerprint(service_key)
    }
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

    /// Commit-guarded Service-map update (ADR-0040 § 2, ADR-0233, R-59, R-60).
    ///
    /// Everything below runs under the `service_apply_commit` write guard, so
    /// `selected_backend_identity` readers wait for it. A non-empty update:
    ///
    /// 1. Plan (`plan_service_apply`), before any mutation: validate and
    ///    deduplicate the backends, reject one endpoint carrying two
    ///    allocations, reject a reverse-NAT key another Service's VIP already
    ///    owns ([`DataplaneError::ReverseNatConflict`], R-59) and an identity
    ///    conflict ([`DataplaneError::BackendIdentityConflict`]); then mint ids
    ///    for endpoints the allocator has not seen and reserve their identities
    ///    (not yet readable).
    /// 2. Populate `BACKEND_MAP` (an identical entry is reused, a differing
    ///    `weight`/`healthy` is updated in place with the prior value saved),
    ///    build the fresh private inner map holding the Maglev permutation, and
    ///    insert the reverse-NAT entries. The outer pointer is untouched, so no
    ///    new id is selectable yet. A kernel rejection while allocating the
    ///    inner map is [`DataplaneError::MapAllocFailed`] (S-2.2-11).
    /// 3. Commit with the single atomic `SERVICE_MAP` outer
    ///    `set(&service_key, new_inner.as_fd())`; the kernel ref-counts the old
    ///    and new inner maps, so concurrent XDP readers see one or the other.
    /// 4. Publish every reserved identity as `Applied`, update the trackers and
    ///    release the guard.
    /// 5. Post-commit orphan cleanup: sweep `BACKEND_MAP` and the stale
    ///    reverse-NAT keys. A key that cannot be removed becomes cleanup debt
    ///    retried by the next update; it never turns the committed update into
    ///    a failure.
    ///
    /// A failure before step 3 replays the undo log: it deletes only keys that
    /// were absent before the update, restores each in-place `BACKEND_MAP`
    /// update from its saved prior value, removes the reserved identities and
    /// releases the fresh allocator memos. The original stage error is
    /// returned, or [`DataplaneError::ServiceApplyRollback`] when a restore (a
    /// delete or an in-place restore, R-60) also failed. An empty update
    /// deletes the outer slot (the commit) and then cleans up.
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
        use crate::maps::wire::ServiceKey;

        // `frontend.vip` is IPv4-by-construction (ADR-0060 D1a) — narrow
        // infallibly. `frontend.proto()` is the single declared L4
        // protocol; the REVERSE_NAT fan-out is per-proto (D4).
        let vip = frontend.vip_v4();
        let svc_proto = frontend.proto();
        // The outer slot is keyed on the declared frontend `(vip, port,
        // proto)` (ADR-0060 site #7): a UDP service keys a distinct slot
        // (C3 guard), and `frontend.port()` — not a backend's listener port —
        // is the VIP port the XDP program matches on.
        let service_key = ServiceKey {
            vip_host: u32::from(vip),
            port_host: frontend.port().get(),
            proto: svc_proto.as_u8(),
            _pad: 0,
        };

        // Commit guard (ADR-0233, ADR-0241): update writers serialise and
        // `selected_backend_identity` readers wait for it. From here to the end
        // of the function the body is synchronous aya map syscalls and
        // in-memory mutation with no `.await`, so cancellation cannot expose a
        // half-published state.
        let _commit = self.service_apply_commit.write();

        if backends.is_empty() {
            return self.withdraw_service(service_key);
        }
        // The fingerprint of the slice exactly as given: never sorted, never
        // recomputed from a tracker (D-20).
        let applied =
            overdrive_core::dataplane::fingerprint::fingerprint(&frontend.vip(), &backends);
        self.apply_service(service_key, svc_proto, &backends, applied)
    }

    /// see #27 (telemetry `ringbuf`)
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the native→generic fallback classification
    //! helper (S-2.2-02) and the `classify_attach_result` dispatcher.

    use super::DataplaneError;
    use overdrive_testing::placement;

    placement!(host_pure);

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

    /// `cgroup_load_error` builds a [`DataplaneError::LoadFailed`] whose
    /// message names the hook and the `load` phase.
    #[test]
    fn cgroup_load_error_yields_load_failed_naming_hook() {
        use aya::programs::ProgramError;
        use aya::sys::SyscallError;
        use std::io;

        let err = ProgramError::SyscallError(SyscallError {
            call: "bpf_prog_load",
            io_error: io::Error::from_raw_os_error(libc::E2BIG),
        });
        let built = super::cgroup_load_error("cgroup_connect4_gateway", &err);
        match built {
            DataplaneError::LoadFailed(message) => {
                assert!(
                    message.contains("cgroup_connect4_gateway"),
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
}
