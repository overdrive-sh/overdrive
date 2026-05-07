//! `overdrive-dataplane` — userspace BPF loader per ADR-0038.
//!
//! Owns [`EbpfDataplane`], the production binding of the
//! [`Dataplane`] port trait from `overdrive-core`. The kernel-side
//! object produced by `overdrive-bpf` is embedded at compile time via
//! `include_bytes!`; on Linux the loader attaches the `xdp_pass`
//! program to the configured interface. On non-Linux build targets
//! (developer macOS, primarily) the constructor returns
//! [`DataplaneError::LoadFailed`] with a `"non-Linux build target"`
//! diagnostic — the rest of the workspace still compiles.
//!
//! Phase 2.1 step 01-02 ships the loader skeleton. The three trait
//! methods (`update_policy`, `update_service`, `drain_flow_events`)
//! are stubbed pending #158 (`POLICY_MAP`), #24 (`SERVICE_MAP`), and
//! #31 (telemetry ringbuf) per `architecture.md` §7.

#![expect(
    clippy::doc_markdown,
    reason = "Phase 2.2 RED scaffolds in maglev/* and swap.rs; lints self-trip when scaffolds go GREEN. Strip when Slice 08 closes the last scaffold."
)]

// Phase 2.2 module scaffolds per
// `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
// DWD-3 file-path inventory. Bodies panic via `todo!()` until
// DELIVER fills them per the carpaccio slice plan.
pub mod loader;
pub mod maglev;
pub mod maps;
pub mod swap;

// Orphan-GC sweep over `BACKEND_MAP` (step 4 of ADR-0040 § 2's
// 5-step swap orchestration). Linux-only — the module's
// `#![cfg(target_os = "linux")]` elides the body on macOS without
// dragging the cfg gate up here.
pub mod gc;

// Direct `bpf(2)` syscall surface used where aya 0.13.x ships no
// typed wrappers (HASH_OF_MAPS construction + `BPF_PROG_TEST_RUN`).
// Linux-only — gated within the module.
#[cfg(target_os = "linux")]
pub mod sys;

// `Dataplane` trait + supporting types — only used by the Linux-side
// trait impl below. Non-Linux builds get a stub `impl Dataplane`
// (further down) that uses no extra symbols.
#[cfg(target_os = "linux")]
use std::net::Ipv4Addr;

#[cfg(target_os = "linux")]
use async_trait::async_trait;
#[cfg(not(target_os = "linux"))]
use overdrive_core::traits::dataplane::DataplaneError;
#[cfg(target_os = "linux")]
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Embedded kernel-side BPF object. Produced by
/// `cargo xtask bpf-build` (step 02-01) and copied to the stable path
/// `target/xtask/bpf-objects/overdrive_bpf.o`. The `build.rs` shim
/// (step 01-03) converts a missing artifact into a single-line
/// actionable error.
///
/// Lives behind `#[cfg(target_os = "linux")]` so non-Linux builds do
/// not need the artifact present at compile time — the
/// `cfg(not(target_os = "linux"))` `new()` returns an error before
/// any aya code runs.
// `OVERDRIVE_BPF_OBJECT_PATH` is emitted by `build.rs` as a
// `cargo:rustc-env=` directive, resolved at build-script time against
// either the `OVERDRIVE_BPF_OBJECT` override (set by `cargo xtask
// mutants`) or the workspace-relative fallback. Single env-var entry
// point keeps the `include_bytes!` macro independent of how the path
// was computed — see `build.rs` module docstring for the override
// rationale and `xtask::mutants::bpf_object_env_override` for the
// wrapper-side mechanics.
#[cfg(target_os = "linux")]
const OVERDRIVE_BPF_OBJ: &[u8] = include_bytes!(env!("OVERDRIVE_BPF_OBJECT_PATH"));

/// Production bpffs pin directory for SERVICE_MAP and any future
/// HoM-shaped maps. The kernel-side declaration carries
/// `pinning = ByName`; aya's loader joins this directory + the map
/// name when resolving the pre-pinned FD via `BPF_OBJ_GET`. See
/// `.claude/rules/development.md` § "Sharing the outer HoM between
/// userspace and the kernel-side ELF — `pinning = ByName`".
#[cfg(target_os = "linux")]
const DEFAULT_PIN_DIR: &str = "/sys/fs/bpf/overdrive";

/// SERVICE_MAP outer-map name. MUST match the `#[map]` `export_name`
/// emitted from `crates/overdrive-bpf/src/maps/service_map.rs` —
/// that name is what aya's loader uses to join `<pin_dir>/<name>`.
#[cfg(target_os = "linux")]
const SERVICE_MAP_NAME: &str = "SERVICE_MAP";

/// BACKEND_MAP name — regular HASH map; aya supports it natively
/// (no pin-by-name workaround needed).
#[cfg(target_os = "linux")]
const BACKEND_MAP_NAME: &str = "BACKEND_MAP";

/// REVERSE_NAT_MAP name — regular HASH map keyed on
/// `BackendKeyPod { ip_host, port_host, proto, _pad }` →
/// `VipPod { ip_host, port_host, _pad }`. aya supports HASH natively
/// (Slice 05-04: promoted from in-memory placeholder per
/// `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`).
#[cfg(target_os = "linux")]
const REVERSE_NAT_MAP_NAME: &str = "REVERSE_NAT_MAP";

/// SERVICE_MAP outer-map capacity in services. 4096 per
/// architecture.md § 10. MUST match the kernel-side
/// `MAX_OUTER_ENTRIES` const in `service_map.rs` — kernel and
/// userspace see the same map (pin-by-name shares the FD), so the
/// capacities are consistent by definition.
#[cfg(target_os = "linux")]
const SERVICE_MAP_OUTER_CAPACITY: u32 = 4096;

/// SERVICE_MAP inner-ARRAY size in slots. Equals
/// [`overdrive_core::dataplane::MaglevTableSize::DEFAULT`].get() = 16_381
/// per architecture.md § 5 Q-Sig D6 / ADR-0041 — the Maglev table
/// size. **MUST** stay in lockstep with `INNER_TABLE_SIZE` in
/// `crates/overdrive-bpf/src/maps/service_map.rs` (kernel-side); a
/// drift between the two would silently misroute packets via slot
/// out-of-bounds reads (the kernel ARRAY map clamps to its declared
/// size; userspace populating slots beyond it is a no-op).
#[cfg(target_os = "linux")]
const SERVICE_MAP_INNER_CAPACITY: u32 = overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();

/// Production dataplane.
///
/// Loads `overdrive_bpf.o`, pre-creates and pre-pins the `SERVICE_MAP`
/// outer HASH_OF_MAPS so aya's loader reuses the FD via
/// `pinning = ByName` (per `.claude/rules/development.md`
/// § "Sharing the outer HoM between userspace and the kernel-side
/// ELF — `pinning = ByName`"), and attaches the configured XDP
/// program to the requested interface.
#[cfg(target_os = "linux")]
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
    /// purge invariant). Tracking per-service prevents accidental
    /// cross-service deletion when two services briefly share a
    /// backend address.
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
}

#[cfg(not(target_os = "linux"))]
pub struct EbpfDataplane;

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
    #[cfg(target_os = "linux")]
    pub fn new_with_pin_dir(
        client_iface: &str,
        backend_iface: &str,
        pin_dir: &std::path::Path,
    ) -> Result<Self, DataplaneError> {
        use aya::programs::{Xdp, XdpFlags};
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
                return Err(DataplaneError::LoadFailed(format!(
                    "xdp_reverse_nat_lookup.attach({backend_iface}, DRV_MODE): {e}"
                )));
            }
        };

        Ok(Self {
            bpf,
            service_map,
            backend_map: parking_lot::Mutex::new(backend_map),
            reverse_nat_map: parking_lot::Mutex::new(reverse_nat_map),
            service_backends: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            service_reverse_nat_keys: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            pin_dir: pin_dir.to_path_buf(),
            _xdp_forward_link: xdp_forward_link,
            _xdp_reverse_link: xdp_reverse_link,
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
    #[cfg(target_os = "linux")]
    pub fn new(client_iface: &str, backend_iface: &str) -> Result<Self, DataplaneError> {
        Self::new_with_pin_dir(client_iface, backend_iface, std::path::Path::new(DEFAULT_PIN_DIR))
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
    #[cfg(target_os = "linux")]
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
    /// Phase 2.2 hardcodes proto = TCP; UDP support follows in a
    /// future slice when the trait surface gains the field.
    ///
    /// # Errors
    ///
    /// Returns [`DataplaneError::LoadFailed`] if the kernel rejects
    /// the lookup with anything other than `KeyNotFound` (which is
    /// the `Ok(false)` path).
    #[cfg(target_os = "linux")]
    pub fn reverse_nat_map_has_backend(
        &self,
        ip: Ipv4Addr,
        port: u16,
    ) -> Result<bool, DataplaneError> {
        use crate::maps::BackendKeyPod;
        use overdrive_core::dataplane::backend_key::Proto;

        let key = BackendKeyPod {
            ip_host: u32::from(ip),
            port_host: port,
            proto: Proto::Tcp.as_u8(),
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
    #[cfg(target_os = "linux")]
    pub fn backend_map_keys(&self) -> Result<Vec<u32>, DataplaneError> {
        let backend_map = self.backend_map.lock();
        backend_map
            .keys()
            .collect::<Result<Vec<u32>, _>>()
            .map_err(|e| DataplaneError::LoadFailed(format!("BACKEND_MAP keys(): {e}")))
    }

    /// Non-Linux fallthrough — returns
    /// [`DataplaneError::LoadFailed`] with a `"non-Linux build
    /// target"` diagnostic. Lets the rest of the workspace compile on
    /// macOS without aya in the dep graph (architecture.md §5.2).
    #[cfg(not(target_os = "linux"))]
    pub fn new(_client_iface: &str, _backend_iface: &str) -> Result<Self, DataplaneError> {
        Err(DataplaneError::LoadFailed("overdrive-dataplane: non-Linux build target".into()))
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
#[cfg(target_os = "linux")]
fn should_fallback_to_generic(io_error: &std::io::Error) -> bool {
    io_error.raw_os_error().is_some_and(|code| code == libc::EOPNOTSUPP)
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
#[async_trait]
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
    /// VIP port note: the `Dataplane` trait passes a single
    /// `Ipv4Addr` plus a `Vec<Backend>`. Slice 03 derives the VIP
    /// port from `backends[0].addr.port()` (matches the Slice 02
    /// convention) — every backend in a set serves the same VIP
    /// port. Slice 04 lifts a separate VIP-port parameter through
    /// the trait (architecture.md § 5 D-Sig).
    async fn update_service(
        &self,
        vip: Ipv4Addr,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        use std::os::fd::AsFd;

        use crate::maps::wire::{BackendEntryPod, ServiceKey};

        // Empty backend set — clear the outer slot. (Trait does not
        // distinguish "no backends" from "remove service"; equivalent
        // semantics here: drop the slot so XDP returns XDP_PASS for
        // this VIP.)
        if backends.is_empty() {
            // No VIP port available; nothing to clear.
            return Ok(());
        }

        use crate::maps::{BackendKeyPod, VipPod};
        use overdrive_core::dataplane::backend_key::Proto;

        let vip_port = backends[0].addr.port();
        let service_key = ServiceKey { vip_host: u32::from(vip), port_host: vip_port, _pad: 0 };

        // Step 1 — Upsert each backend into BACKEND_MAP. BackendId
        // is derived from (IPv4, port) — host-order u32 of the IP
        // shifted up by 16 bits, OR'd with the port, giving a stable
        // 48-bit-shaped identifier in a 32-bit space (the high 16
        // bits of the IP collide with the port, which is acceptable
        // for the slot-set contract — slot lookups go through the
        // inner ARRAY, not BackendId reverse-mapping). A future
        // BackendId allocator can replace this; the swap shape
        // doesn't depend on the specific derivation.
        let mut backend_ids: Vec<u32> = Vec::with_capacity(backends.len());
        {
            // Lock is held only for the BACKEND_MAP populate loop;
            // dropped at end of this block (the `{ ... }` braces) before
            // any further work. The fn-level `#[allow(
            // clippy::significant_drop_tightening)]` covers this.
            let mut backend_map = self.backend_map.lock();
            for backend in &backends {
                let pod = BackendEntryPod::from_backend(backend)?;
                // Use a deterministic ID derived from IP+port. Same
                // (ip, port) yields the same BackendId across
                // updates; orphan GC removes IDs not in the new set.
                let backend_id: u32 = pod
                    .ipv4_host
                    .wrapping_mul(2_654_435_761)
                    .wrapping_add(u32::from(pod.port_host));
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
            .filter_map(|backend| {
                BackendEntryPod::from_backend(backend).ok().and_then(|pod| {
                    let bid_raw: u32 = pod
                        .ipv4_host
                        .wrapping_mul(2_654_435_761)
                        .wrapping_add(u32::from(pod.port_host));
                    overdrive_core::id::BackendId::new(bid_raw)
                        .ok()
                        .map(|bid| (bid, backend.weight.max(1)))
                })
            })
            .collect();
        let permutation = crate::maglev::permutation::generate(
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
            crate::gc::sweep_orphan_backends(&mut backend_map, &live_ids).map_err(|e| {
                DataplaneError::LoadFailed(format!("BACKEND_MAP orphan-GC sweep: {e}"))
            })?;
        }

        // Step 4b — REVERSE_NAT_MAP lockstep populate + purge
        // (Slice 05-04, S-2.2-18).
        //
        // For every backend in the new set, install
        // `(backend_ip, backend_port, proto=TCP)` → `(vip_ip, vip_port)`
        // so the `xdp_reverse_nat_lookup` program (attached on the
        // backend-facing veth ingress per ADR-0045) can rewrite
        // the source 5-tuple of response packets back to the VIP.
        //
        // For backends that were in the PRIOR set but are not in
        // the new set, delete the corresponding REVERSE_NAT_MAP
        // entry — without this, a late response from a removed
        // backend would still be rewritten to the VIP, leaking
        // service identity across removals (the architectural
        // invariant S-2.2-18 pins).
        //
        // Phase 2.2 hardcodes `proto = TCP` because the trait
        // surface does not yet carry per-backend protocol; UDP
        // services would lift this once the trait gains the field.
        // Per `.claude/rules/development.md` § "Persist inputs, not
        // derived state" — the per-service tracker carries the
        // BackendKeyPods themselves (the authoritative inputs), not
        // a derived "should-be-deleted" flag.
        let new_keys: std::collections::BTreeSet<BackendKeyPod> = backends
            .iter()
            .filter_map(|backend| match backend.addr.ip() {
                std::net::IpAddr::V4(v4) => Some(BackendKeyPod {
                    ip_host: u32::from(v4),
                    port_host: backend.addr.port(),
                    proto: Proto::Tcp.as_u8(),
                    _pad: 0,
                }),
                std::net::IpAddr::V6(_) => None,
            })
            .collect();
        let vip_value = VipPod { ip_host: u32::from(vip), port_host: vip_port, _pad: 0 };
        let prior_keys: std::collections::BTreeSet<BackendKeyPod> = {
            let mut tracker = self.service_reverse_nat_keys.lock();
            // Snapshot the prior set BEFORE overwriting; the diff
            // (`prior - new`) drives the purge below.
            let prior = tracker.get(&service_key).cloned().unwrap_or_default();
            tracker.insert(service_key, new_keys.clone());
            prior
        };
        {
            let mut reverse_nat_map = self.reverse_nat_map.lock();
            // Insert / update every key in the new set. `insert`
            // with flags=0 (BPF_ANY) accepts both "new" and
            // "existing" — idempotent.
            for key in &new_keys {
                reverse_nat_map.insert(key, vip_value, 0).map_err(|e| {
                    DataplaneError::LoadFailed(format!("REVERSE_NAT_MAP insert: {e}"))
                })?;
            }
            // Purge keys that were in the prior set but are not in
            // the new set. `remove` returns `Err(KeyNotFound)` for
            // entries already gone — fold into Ok().
            for stale in prior_keys.difference(&new_keys) {
                match reverse_nat_map.remove(stale) {
                    Ok(()) => {}
                    Err(aya::maps::MapError::KeyNotFound) => {}
                    Err(e) => {
                        return Err(DataplaneError::LoadFailed(format!(
                            "REVERSE_NAT_MAP purge: {e}"
                        )));
                    }
                }
            }
        }

        // Step 5 — Old inner map released by aya's ref-counting once
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
}

#[cfg(test)]
mod tests {
    //! macOS-side regression guards for the `#[cfg(not(target_os =
    //! "linux"))]` stub branch, plus Linux-side unit tests for the
    //! native→generic fallback classification helper (S-2.2-02).
    //!
    //! The macOS branch is one line of code, but the test exists to
    //! prevent silent erosion of the boundary — a future refactor
    //! that drops the cfg gate, weakens the diagnostic, or returns
    //! a different error variant trips this assertion on macOS CI
    //! before the change reaches Linux.
    //!
    //! On Linux the macOS test is `#[cfg(not(target_os = "linux"))]`-
    //! gated and silently absent — the Tier 3 LVH smoke (`cargo xtask
    //! integration-test vm latest`, step 03-02) is the corresponding
    //! Linux-side gate. The fallback-classification unit tests below
    //! run on Linux only (the helper itself is `#[cfg(target_os =
    //! "linux")]`).

    // Imports are only consumed by the `#[cfg(not(target_os =
    // "linux"))]` test below, so they're dead on Linux. The cfg gate
    // can't sit on `use` directly without complicating the macOS
    // path; allowing here keeps both paths clean.
    #[cfg(not(target_os = "linux"))]
    use super::{DataplaneError, EbpfDataplane};

    /// On non-Linux build targets the constructor returns
    /// [`DataplaneError::LoadFailed`] carrying the `"non-Linux build
    /// target"` diagnostic — never any other variant, never a
    /// surprise `Ok(_)`.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn new_returns_load_failed_with_non_linux_diagnostic() {
        // `EbpfDataplane` does not implement `Debug` (its inner aya
        // types do not, and adding a manual impl is noise for a stub
        // that lives only on Linux). Unwrap the `Result` via match
        // rather than `expect_err`, which would require `T: Debug`.
        match EbpfDataplane::new("lo", "lo") {
            Err(DataplaneError::LoadFailed(msg)) => {
                assert!(msg.contains("non-Linux build target"), "unexpected diagnostic: {msg}");
            }
            Err(other) => panic!("expected DataplaneError::LoadFailed, got {other:?}"),
            Ok(_) => panic!("expected Err on non-Linux build target"),
        }
    }

    /// Classification — `EOPNOTSUPP` from `bpf_link_create` /
    /// `netlink_set_xdp_fd` is the canonical "driver does not
    /// support native XDP" signal. Trigger fallback to generic
    /// (`SKB_MODE`).
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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
}
