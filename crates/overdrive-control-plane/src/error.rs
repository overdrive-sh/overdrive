//! `ControlPlaneError` — top-level typed error with pass-through `#[from]`.
//!
//! Per ADR-0015, one top-level enum. Exhaustive `to_response` function
//! maps every variant to `(StatusCode, Json<ErrorBody>)`. Body shape is
//! a deliberate RFC 7807-compatible subset so v1.1 upgrade is additive.

use std::fmt;
use std::path::PathBuf;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::ca::CaError;

use crate::api::ErrorBody;
use crate::view_store::{ProbeError, ViewStoreError};

/// Boot-time failures from the runtime-owned `ViewStore` (ADR-0035 §4).
///
/// Distinct typed variant per failure mode so the composition root in
/// `overdrive-cli::commands::serve` can branch on `matches!(...)`
/// without `Display`-grepping a stringified `Internal` message. The
/// boot path emits `health.startup.refused` when this variant fires;
/// see `ControlPlaneError::ViewStoreBoot`.
///
/// Pass-through embedding via `#[source]` per
/// `.claude/rules/development.md` § Errors — preserves the structured
/// `ViewStoreError` / `ProbeError` chain through audit logs and the
/// §12 investigation agent instead of stringifying it.
#[derive(Debug, Error)]
pub enum ViewStoreBootError {
    /// `RedbViewStore::open` failed at the production boot path.
    /// Typical causes: missing parent directory create, redb file
    /// corruption, concurrent open in the same process.
    #[error("open RedbViewStore at {path}: {source}")]
    Open {
        /// The resolved redb file path the open targeted.
        path: PathBuf,
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },

    /// Earned-Trust startup probe failed during `register`. The
    /// composition root short-circuits boot with `health.startup.refused`
    /// before any reconciler enters the registry.
    #[error("probe failed for reconciler {reconciler}: {source}")]
    Probe {
        /// Name of the reconciler whose `register` call surfaced the
        /// probe failure. Probe is per-call (not per-runtime), so the
        /// failing reconciler is the one that triggered the probe.
        reconciler: ReconcilerName,
        /// Underlying `ProbeError` cause.
        #[source]
        source: ProbeError,
    },

    /// `bulk_load` round-trip failed during `register` (CBOR decode
    /// error or underlying I/O failure). Hard boot failure — the
    /// composition root refuses to come up.
    #[error("bulk_load failed for reconciler {reconciler}: {source}")]
    BulkLoad {
        /// Name of the reconciler whose `register` call attempted the
        /// `bulk_load` round-trip.
        reconciler: ReconcilerName,
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },
}

/// Boot-time failures from the cgroup-bootstrap surface
/// (`cgroup_manager::create_and_enrol_control_plane_slice_at` plus
/// the future `overdrive-worker::cgroup_manager::
/// create_workloads_slice_with_controllers`).
///
/// Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
/// § "Production fix #1 — Typed errors", the
/// `overdrive.slice/cgroup.subtree_control` write surfaces as a
/// discrete error variant per `.claude/rules/development.md`
/// § "Distinct failure modes get distinct error variants" — never
/// absorbed into a generic `io::Error`. EBUSY (a process is already
/// in the cgroup; the slice was previously initialised in the wrong
/// order) and "any other I/O error" carry distinct operator
/// remediation hints.
///
/// Pass-through embedding via `#[source]` per
/// `.claude/rules/development.md` § Errors — preserves the structured
/// `io::Error` chain (`ErrorKind::ResourceBusy`, `kind`, `os_error`)
/// through audit logs and operator-facing diagnostics instead of
/// stringifying it.
///
/// **Step 01-01 NOTE**: this enum is declared but no production call
/// site constructs its variants yet. Step 01-02 wires
/// `create_and_enrol_control_plane_slice_at` and the new
/// `create_workloads_slice_with_controllers` to `map_err(...)?` into
/// these variants. Until then the variants are unreachable from
/// production; they exist as the typed contract the RED regression
/// test (`tests/integration/cgroup_isolation/
/// alloc_scope_has_writable_cpu_weight_and_memory_max.rs`) pins.
#[derive(Debug, Error)]
pub enum CgroupBootstrapError {
    /// The kernel returned EBUSY on the
    /// `overdrive.slice/cgroup.subtree_control` write — a process is
    /// already enrolled in `overdrive.slice` (the slice was
    /// previously initialised in the wrong order, with a process
    /// added to `cgroup.procs` BEFORE the controllers were enabled).
    /// Per cgroup v2 contract a cgroup with a process in
    /// `cgroup.procs` cannot have its parent's `subtree_control`
    /// modified; the kernel response is `EBUSY`.
    ///
    /// Operator hint: restart the server cleanly so no stale process
    /// is left in `overdrive.slice`. If the leak persists across
    /// restarts, run the leftover-cgroup cleanup discipline from
    /// `.claude/rules/testing.md` § "Leaked workload cgroups across
    /// runs".
    #[error(
        "cgroup `subtree_control` write rejected with EBUSY — a process \
         is already enrolled in overdrive.slice (controllers must be \
         enabled BEFORE any process is enrolled).\n\
         \n\
         Try: restart the server cleanly so no stale process is left \
         in overdrive.slice. If a stale process persists across \
         restarts, sweep leftover cgroups per the leftover-cgroup \
         cleanup discipline.\n\
         \n\
         Underlying: {source}"
    )]
    SubtreeControlBusy {
        /// Underlying `io::Error` carrying `ErrorKind::ResourceBusy`
        /// (or the equivalent `raw_os_error() == EBUSY`).
        #[source]
        source: std::io::Error,
    },

    /// Catch-all I/O failure on the
    /// `overdrive.slice/cgroup.subtree_control` (or analogous child)
    /// write that is NOT EBUSY. Any other `ErrorKind` — typically
    /// `PermissionDenied` (`EACCES` from cgroupfs delegation refusal),
    /// `NotFound` (the enclosing slice does not exist), or
    /// `InvalidInput` (the kernel rejected the controller list) —
    /// flows here.
    ///
    /// Operator hint: inspect cgroupfs delegation for the enclosing
    /// slice. The pre-flight at `cgroup_preflight.rs` should have
    /// caught the most common shapes (no delegation, missing `cpu`
    /// controller); a failure here typically means the pre-flight
    /// passed but the runtime delegation surface differs (e.g. a
    /// systemd unit replaced the slice between pre-flight and
    /// bootstrap).
    #[error(
        "cgroup `subtree_control` write failed: {source}\n\
         \n\
         Try: inspect cgroupfs delegation for the enclosing slice — \
         the pre-flight passed, so this is typically a runtime \
         divergence (a systemd unit replaced the slice between \
         pre-flight and bootstrap, or a concurrent operator action \
         removed delegation)."
    )]
    SubtreeControlWriteFailed {
        /// Underlying `io::Error` for any non-EBUSY I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// I/O failure on a non-`subtree_control` bootstrap operation —
    /// `mkdir` of the slice directory or `cgroup.procs` write for PID
    /// enrolment. Distinct from [`SubtreeControlWriteFailed`] so the
    /// operator message names the actual operation that failed, not a
    /// file that may not even exist yet.
    ///
    /// Mirrors [`WorkloadsBootstrapError::WriteFailed`] in the worker
    /// crate, which uses the same generic "bootstrap failed" message
    /// for non-subtree_control operations.
    #[error(
        "cgroup bootstrap failed: {source}\n\
         \n\
         Try: verify cgroupfs is mounted at /sys/fs/cgroup and the \
         running process has permission to create directories and \
         write cgroup.procs under overdrive.slice."
    )]
    BootstrapIoFailed {
        /// Underlying `io::Error` from `mkdir` or `cgroup.procs` write.
        #[source]
        source: std::io::Error,
    },
}

impl CgroupBootstrapError {
    /// Construct a [`CgroupBootstrapError`] from an `io::Error` returned
    /// by a `subtree_control` write, dispatching on `ErrorKind` so
    /// EBUSY surfaces as the discrete
    /// [`CgroupBootstrapError::SubtreeControlBusy`] variant and
    /// everything else collapses into
    /// [`CgroupBootstrapError::SubtreeControlWriteFailed`].
    ///
    /// Production call sites (landing in step 01-02) use this
    /// constructor via `.map_err(CgroupBootstrapError::from_subtree_control_io)?`
    /// rather than spelling out the match at every site. Keeping the
    /// dispatch in one place makes the EBUSY-vs-other discrimination a
    /// single line in production and trivially mockable from tests.
    #[must_use]
    pub fn from_subtree_control_io(source: std::io::Error) -> Self {
        // `ErrorKind::ResourceBusy` is the stable shape (Rust 1.83+);
        // `raw_os_error() == Some(libc::EBUSY)` is the structural
        // fallback for older toolchains and for io::Errors constructed
        // without the kind hint (e.g. from `tokio::fs` paths that lose
        // the kind on conversion).
        let is_ebusy = matches!(source.kind(), std::io::ErrorKind::ResourceBusy)
            || source.raw_os_error() == Some(libc::EBUSY);
        if is_ebusy {
            Self::SubtreeControlBusy { source }
        } else {
            Self::SubtreeControlWriteFailed { source }
        }
    }
}

/// Boot-time failures from the `EbpfDataplane` composition path per
/// architecture.md § 5.3 of
/// `backend-discovery-bridge-service-reachability`.
///
/// Three discrete failure modes — `Construct`, `Probe`,
/// `IfaceAddrResolution` — per `.claude/rules/development.md`
/// § Errors → "Distinct failure modes get distinct error variants".
/// Each variant's `#[error("...")]` template embeds the operator-
/// facing remediation steps from architecture.md § 5.3 verbatim
/// (`ip link show <iface>`, `mount | grep bpffs`, `dmesg | tail`,
/// `ip -4 addr show <iface>`, CAP_SYS_ADMIN / CAP_BPF check).
///
/// **Step 02-01 NOTE**: only the `IfaceAddrResolution` variant is
/// reachable from production today — the `Construct` and `Probe`
/// variants land alongside `EbpfDataplane` boot composition in step
/// 02-02. They are declared here so the typed contract is in place
/// and the boot_composition.rs RED scaffolds at S-BDB-13, S-BDB-14
/// can branch on `matches!(e, DataplaneBootError::Construct { .. })`
/// when their bodies go GREEN.
#[derive(Debug, Error)]
pub enum DataplaneBootError {
    /// `EbpfDataplane::new` construction failed. Typical causes: the
    /// configured interface does not exist, /sys/fs/bpf is not
    /// mounted, the kernel rejected the BPF program at load time, the
    /// process lacks `CAP_SYS_ADMIN` / `CAP_BPF`. Operator hint
    /// embeds the canonical inspection commands verbatim.
    ///
    /// Reachable from step 02-02 onwards; declared in 02-01 for the
    /// typed contract surface.
    #[error(
        "EbpfDataplane construction failed \
         (client_iface={client_iface}, backend_iface={backend_iface}): {source}\n\
         \n\
         Try:\n\
           - `ip link show <iface>` to verify the interface exists.\n\
           - `mount | grep bpffs` to verify /sys/fs/bpf is mounted.\n\
           - `dmesg | tail` for kernel-side BPF verifier errors.\n\
           - Confirm CAP_SYS_ADMIN / CAP_BPF for the running process."
    )]
    Construct {
        /// Operator-supplied `[dataplane] client_iface` value.
        client_iface: String,
        /// Operator-supplied `[dataplane] backend_iface` value.
        backend_iface: String,
        /// Underlying `DataplaneError` from `EbpfDataplane::new`.
        #[source]
        source: overdrive_core::traits::dataplane::DataplaneError,
    },

    /// Earned-Trust probe per architecture.md § 5.4 failed — the
    /// kernel accepted the BPF programs at load time but the runtime
    /// BACKEND_MAP write+read sentinel did not round-trip. Typically
    /// indicates a kernel BPF feature regression or a corrupted
    /// bpffs pin from a prior unclean shutdown.
    ///
    /// Reachable from step 02-02 onwards; declared in 02-01 for the
    /// typed contract surface.
    #[error(
        "EbpfDataplane probe failed — the kernel accepted the BPF \
         programs at load time but the runtime BACKEND_MAP write+read \
         did not round-trip. This typically indicates a kernel BPF \
         feature regression or a corrupted bpffs pin from a prior \
         unclean shutdown.\n\
         \n\
         Try:\n\
           - `rm /sys/fs/bpf/overdrive/*` and retry.\n\
           - Inspect `dmesg | tail`.\n\
         \n\
         Underlying: {source}"
    )]
    Probe {
        /// Underlying `DataplaneError` from `EbpfDataplane::probe`.
        #[source]
        source: overdrive_core::traits::dataplane::DataplaneError,
    },

    /// The single-node veth provisioner (ADR-0061 § 3, step 01-03)
    /// failed to stand up the host-netns veth pair before
    /// `EbpfDataplane::new`. Reached only on the production
    /// (non-`dataplane_override`) boot branch AND only when the
    /// configured ifaces are the default veth names — an operator who
    /// names real NICs skips provision entirely, so this variant cannot
    /// fire on the two-NIC path.
    ///
    /// Pass-through `#[from]` per `.claude/rules/development.md`
    /// § "Never flatten a typed error to Internal(String)": the
    /// underlying [`crate::veth_provisioner::VethProvisionError`]
    /// carries a distinct variant per failing `ip(8)` step
    /// (link-show / link-add / addr-add / link-up / route-add), so the
    /// CLI / §12 investigation agent can branch on which provisioning
    /// step failed without `Display`-grepping. Mirrors the `Construct`
    /// / `Probe` precedent above.
    #[error("single-node veth provisioning failed: {source}")]
    Provision {
        /// Underlying typed provisioner failure.
        #[from]
        source: crate::veth_provisioner::VethProvisionError,
    },

    /// `iface::resolve_iface_ipv4` failed for the configured
    /// `client_iface`. Two sub-cases collapse into one variant
    /// because the operator remediation (`ip -4 addr show <iface>`)
    /// is identical: the iface does not exist, OR the iface exists
    /// but has no IPv4 address bound. The structured `io::ErrorKind`
    /// (`NotFound` vs `Other`) is preserved on `source` for
    /// programmatic inspection by the §12 investigation agent.
    #[error(
        "EbpfDataplane iface IPv4 resolution failed for {iface}: {source}\n\
         \n\
         Try: `ip -4 addr show <iface>` to inspect the interface \
         configuration. The bridge requires a single IPv4 address on \
         the configured client_iface for endpoint derivation."
    )]
    IfaceAddrResolution {
        /// The interface name `resolve_iface_ipv4` was asked to
        /// resolve.
        iface: String,
        /// Underlying `io::Error` carrying `ErrorKind::NotFound` (no
        /// IPv4 binding) or `ErrorKind::Other` (`getifaddrs` system
        /// failure).
        #[source]
        source: std::io::Error,
    },
}

/// Boot-time failure of the production transparent-mTLS layer
/// (transparent-mtls-host-socket, D-MTLS-17, GH #26; step 06-03).
///
/// Mirrors [`DataplaneBootError`]'s `Construct`/`Probe` shape: the mTLS
/// layer is wired AFTER `IdentityMgr` (so `HostMtlsEnforcement` can read
/// the held identity), `probe()`d under the wire→probe→use invariant, and
/// only then used. A failure happens BEFORE the listener binds, so the
/// `to_response` arm on the embedding [`ControlPlaneError::MtlsBoot`]
/// variant is exhaustiveness-only. Pass-through `#[from]` per
/// `.claude/rules/development.md` § "Never flatten a typed error to
/// `Internal(String)`": each cause keeps its own variant so the
/// composition root can `matches!(e, ControlPlaneError::MtlsBoot(_))` for
/// structured boot diagnostics without `Display`-grepping.
#[derive(Debug, Error)]
pub enum MtlsBootError {
    /// `MtlsDataplane::load` failed — the shared `overdrive_bpf.o` could
    /// not be loaded, `cgroup_connect4_mtls` / `MTLS_REDIRECT_DEST` was
    /// absent (a build/embed regression), or the program's verifier load
    /// was rejected. The node MUST refuse to start (fail-closed for
    /// confidentiality — NO degrade to a cleartext path).
    #[error(
        "transparent-mTLS dataplane load failed; refusing to boot \
         (no cleartext fallback): {source}\n\
         \n\
         Try:\n\
           - `mount | grep bpffs` to verify /sys/fs/bpf is mounted.\n\
           - `dmesg | tail` for kernel-side BPF verifier errors.\n\
           - Confirm CAP_BPF / CAP_NET_ADMIN for the running process."
    )]
    Load {
        /// Underlying typed `MtlsDataplaneError` from `MtlsDataplane::load`.
        #[from]
        source: overdrive_dataplane::mtls::MtlsDataplaneError,
    },

    /// `MtlsEnforcement::probe` failed — the kTLS-arm + agent-light
    /// forward-encrypt substrate did not round-trip clean on the loopback
    /// sentinel (D-MTLS-11/12). The proxy is not trustworthy; the node
    /// MUST refuse to start with `health.startup.refused` rather than
    /// degrade to cleartext (fail-closed).
    #[error(
        "transparent-mTLS proxy probe failed; refusing to boot \
         (no cleartext fallback): {source}"
    )]
    Probe {
        /// Underlying `MtlsEnforcementError` from `MtlsEnforcement::probe`.
        #[source]
        source: overdrive_core::traits::mtls_enforcement::MtlsEnforcementError,
    },
}

/// Top-level control-plane error.
#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("validation: {field:?}: {message}")]
    Validation { message: String, field: Option<String> },

    #[error("not found: {resource}")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error(transparent)]
    Intent(#[from] overdrive_core::traits::intent_store::IntentStoreError),

    #[error(transparent)]
    Observation(#[from] overdrive_core::traits::observation_store::ObservationStoreError),

    #[error(transparent)]
    Aggregate(#[from] overdrive_core::aggregate::AggregateError),

    /// TLS-bootstrap failure (cert mint, trust-triple I/O, PEM parse,
    /// rustls config). Pass-through embedding per ADR-0015 §Consequences:
    /// preserves the structured upstream chain (`rcgen::Error`,
    /// `io::Error`, `toml::de::Error`, `base64::DecodeError`,
    /// `rustls::Error`) for audit logs and the §12 investigation agent
    /// instead of stringifying it through [`ControlPlaneError::Internal`].
    /// Maps to `500 Internal` on the wire — TLS bootstrap is infra
    /// failure (ADR-0015 §4 Status-code matrix).
    #[error(transparent)]
    Tls(#[from] crate::tls_bootstrap::TlsBootstrapError),

    /// Pre-flight cgroup v2 delegation refusal per ADR-0028.
    /// Surfaced from the boot-path pre-flight as `From` conversion;
    /// rendered to the operator via `Display` (multi-line "what / why /
    /// how to fix" shape per nw-ux-tui-patterns) and never reaches an
    /// HTTP response — the listener doesn't bind on this error.
    #[error(transparent)]
    Cgroup(#[from] crate::cgroup_preflight::CgroupPreflightError),

    /// Control-plane slice bootstrap failure (`cgroup_manager::
    /// create_and_enrol_control_plane_slice`). Pass-through embedding
    /// so callers can `matches!(e, ControlPlaneError::CgroupBootstrap(_))`
    /// for structured startup diagnostics without `Display`-grepping.
    /// Same boot-path shape as `Cgroup` and `ViewStoreBoot`: happens
    /// BEFORE the listener binds, so the `to_response` arm is
    /// exhaustiveness-only.
    #[error(transparent)]
    CgroupBootstrap(#[from] CgroupBootstrapError),

    /// Workloads-slice bootstrap failure (`overdrive_worker::
    /// cgroup_manager::create_workloads_slice_with_controllers`).
    /// Pass-through embedding so callers can
    /// `matches!(e, ControlPlaneError::WorkloadsBootstrap(_))` for
    /// structured startup diagnostics. Same boot-path shape as
    /// `CgroupBootstrap` above.
    #[error(transparent)]
    WorkloadsBootstrap(#[from] overdrive_worker::cgroup_manager::WorkloadsBootstrapError),

    /// Boot-time `node_health` row write failure
    /// (`overdrive_worker::start_local_node`). Pass-through embedding
    /// so the composition root can `matches!(e,
    /// ControlPlaneError::NodeHealthWrite(_))` for structured boot
    /// diagnostics without `Display`-grepping. Same boot-path shape
    /// as `WorkloadsBootstrap` above: happens BEFORE the listener
    /// binds, so the `to_response` arm is exhaustiveness-only.
    /// Per `.claude/rules/development.md` § "Never flatten a typed
    /// error to `Internal(String)` at a composition boundary".
    #[error(transparent)]
    NodeHealthWrite(#[from] overdrive_worker::NodeHealthWriteError),

    /// `ViewStore` boot-time failure per ADR-0035 §5 (Earned Trust).
    /// Pass-through embedding so `overdrive-cli::commands::serve` can
    /// `matches!(e, ControlPlaneError::ViewStoreBoot(_))` to emit the
    /// `health.startup.refused` event without `Display`-grepping a
    /// stringified message. Maps to `500 Internal` on the wire — boot
    /// failures never reach an HTTP response in practice (the listener
    /// has not bound yet); the arm exists for enum exhaustiveness.
    #[error(transparent)]
    ViewStoreBoot(#[from] ViewStoreBootError),

    /// `EbpfDataplane` boot-time failure per
    /// `backend-discovery-bridge-service-reachability` architecture.md
    /// § 5.3 (step 02-01). Pass-through embedding via `#[from]` so
    /// the composition root can `matches!(e,
    /// ControlPlaneError::DataplaneBoot(_))` for structured boot
    /// diagnostics without `Display`-grepping. Same boot-path shape
    /// as `ViewStoreBoot` / `Cgroup` / `CgroupBootstrap`: happens
    /// BEFORE the listener binds, so the `to_response` arm is
    /// exhaustiveness-only.
    #[error(transparent)]
    DataplaneBoot(#[from] DataplaneBootError),

    /// Transparent-mTLS layer boot-time failure
    /// (transparent-mtls-host-socket, D-MTLS-17, GH #26; step 06-03).
    /// Pass-through `#[from]` so the composition root can `matches!(e,
    /// ControlPlaneError::MtlsBoot(_))` for structured boot diagnostics
    /// without `Display`-grepping. Same boot-path shape as
    /// `DataplaneBoot` above: the mTLS layer is wired + probed AFTER
    /// `IdentityMgr` and BEFORE the listener binds, so the `to_response`
    /// arm is exhaustiveness-only. Fail-closed: on this error the node
    /// refuses to start (NO degrade to a cleartext path).
    #[error(transparent)]
    MtlsBoot(#[from] MtlsBootError),

    /// `[dataplane.vip_allocator]` TOML parser refusal per
    /// ADR-0049 § 5b / service-vip-allocator step 02-02. Pass-through
    /// embedding so the CLI / composition root can branch on
    /// `matches!(e, ControlPlaneError::VipAllocatorConfig(_))` for
    /// structured boot diagnostics. Boot-path-only: like the other
    /// pre-listener variants, this never reaches an HTTP response in
    /// practice; the arm in `to_response` exists for enum
    /// exhaustiveness.
    #[error(transparent)]
    VipAllocatorConfig(#[from] crate::vip_allocator_config::VipAllocatorBootError),

    /// Persistent VIP allocator runtime failure surfaced from the
    /// Service-arm `submit_workload` handler per service-vip-allocator
    /// step 02-03d. Pass-through embedding so the typed inner cause
    /// (pool exhaustion, store I/O failure, envelope corruption) is
    /// preserved through to `to_response`, which branches the typed
    /// inner on HTTP status mapping: `Allocator(Exhausted)` →
    /// HTTP 503 with `error = "pool_exhausted"`; other variants fall
    /// through to HTTP 500.
    #[error(transparent)]
    VipAllocator(#[from] overdrive_dataplane::allocators::PersistentAllocatorError),

    /// Service-health-check-probes ProbeRunner Earned-Trust gate
    /// failure per ADR-0054 § 7. Pass-through embedding so the
    /// composition root / CLI can branch on
    /// `matches!(e, ControlPlaneError::ProbeRunnerBoot(_))` for
    /// structured startup diagnostics — and so the typed inner
    /// `ProbeRunnerError` is preserved through to `to_response`
    /// rather than stringified through
    /// [`ControlPlaneError::Internal`]. Same boot-path shape as
    /// `ViewStoreBoot` / `DataplaneBoot`: happens BEFORE the
    /// listener binds; the structured `health.startup.refused`
    /// event fires at the call site that converts the typed error
    /// into this variant.
    #[error(transparent)]
    ProbeRunnerBoot(#[from] ProbeRunnerBootError),

    /// Boot-time listener-fact projection rebuild failure (ADR-0062
    /// § Decision (1); reconciler-listener-fact-view step 01-02). The
    /// boot wiring rebuilds the in-memory [`crate::listener_facts::
    /// ListenerFactStore`] from the intent SSOT immediately after the
    /// allocator's `bulk_load`; if the underlying `IntentStore` scan
    /// fails the control-plane refuses to start.
    ///
    /// Carries the typed [`crate::reconciler_runtime::ConvergenceError`]
    /// boxed rather than by a bare `#[from]`: that enum already carries a
    /// `ViewPersist(ControlPlaneError)` arm, so a `#[from] ConvergenceError`
    /// here would form a recursive type cycle (`ControlPlaneError` ↔
    /// `ConvergenceError`, E0072). `Box` breaks the cycle by giving the
    /// variant a fixed size, while PRESERVING the full typed error — no
    /// fidelity is lost (the previous `String` flatten discarded the
    /// variant; the boxed `ConvergenceError` keeps it `matches!`-able all
    /// the way down). This is a discrete, named, `matches!`-able variant
    /// per `.claude/rules/development.md` § "Never flatten a typed error
    /// to `Internal(String)`", NOT a flatten into
    /// [`ControlPlaneError::Internal`]. Same boot-path shape as
    /// `ViewStoreBoot` / `DataplaneBoot`: happens BEFORE the listener
    /// binds, so the `to_response` arm is exhaustiveness-only.
    #[error("listener-fact projection rebuild failed at boot: {0}")]
    ListenerFactRebuild(Box<crate::reconciler_runtime::ConvergenceError>),

    /// Describe-side internal-invariant violation per ADR-0064 § 4 /
    /// OQ-4: the `describe_workload` handler's read-only
    /// `allocator.get(&spec_digest)` returned `None` for a persisted
    /// `WorkloadIntent::Service`. A persisted-and-describable Service
    /// ALWAYS has an allocated VIP — submit-time admission allocates
    /// before the intent is written (ADR-0049 § 4) and the boot rebuild
    /// re-seeds the allocator memo from the intent SSOT (ADR-0049 § 8).
    /// A miss therefore means one of those invariants was broken; it is
    /// an internal failure, NOT a client-visible 4xx.
    ///
    /// A DEDICATED variant per `.claude/rules/development.md` § "Errors
    /// → Never flatten a typed error to `Internal(String)`" — carries
    /// the lowercase-hex `spec_digest` (the same digest the describe
    /// handler computes for the top-level response field) so an operator
    /// can trace which Service lost its allocator memo without
    /// `Display`-grepping. Maps to HTTP 500 with the `internal` error
    /// kind in [`to_response`].
    #[error(
        "service VIP missing for spec_digest {spec_digest}: the allocator memo \
         has no entry for this persisted Service. Submit-time admission allocates \
         a VIP before the intent is written (ADR-0049 § 4) and the boot rebuild \
         re-seeds the memo from the intent SSOT (ADR-0049 § 8), so this indicates \
         a broken allocate-or-rebuild invariant — not operator-actionable input."
    )]
    ServiceVipMissing {
        /// Lowercase-hex SHA-256 digest of the Service spec — the
        /// describe handler's top-level `spec_digest` string, threaded
        /// through for diagnostics.
        spec_digest: String,
    },

    /// Built-in CA failure surfaced from the boot-time ephemeral workload-CA
    /// composition (ADR-0067 D3 rev 4: `RcgenCa::root` /
    /// `issue_intermediate` / `trust_bundle`). Pass-through embedding via
    /// `#[from]` per `.claude/rules/development.md` § "Never flatten a typed
    /// error to `Internal(String)`": the typed [`CaError`] carries a distinct
    /// variant per failure mode (signing failure, invalid subject, adoption
    /// conflict, tampered envelope, …), so the CLI / §12 investigation agent
    /// can branch on the cause without `Display`-grepping. Same boot-path
    /// shape as `ViewStoreBoot` / `DataplaneBoot`: happens BEFORE the listener
    /// binds, so the `to_response` arm is exhaustiveness-only.
    #[error(transparent)]
    Ca(#[from] CaError),

    /// Persistent built-in-CA boot failure surfaced from the #215 boot-side
    /// wiring (built-in-ca-operator-composition D-OC-5). Pass-through embedding
    /// via `#[from]` per `.claude/rules/development.md` § "Never flatten a typed
    /// error to `Internal(String)`": the typed [`CaBootError`] keeps each boot
    /// cause distinguishable at the composition root — absent-KEK
    /// (`KekUnavailable`) vs envelope-decrypt failure (`EnvelopeDecrypt`,
    /// embedding the typed [`CaError`] cause so wrong-KEK and tampered render
    /// distinct strings) — so the CLI / §12 investigation agent can branch on
    /// the cause without `Display`-grepping. Same boot-path shape as
    /// `ViewStoreBoot` / `DataplaneBoot`: happens BEFORE the listener binds, so
    /// the `to_response` arm is exhaustiveness-only.
    #[error(transparent)]
    CaBoot(#[from] crate::ca_boot::CaBootError),

    #[error("internal: {0}")]
    Internal(String),
}

/// Service-health-check-probes ProbeRunner Earned-Trust boot
/// failure per ADR-0054 § 7. Wraps the typed `ProbeRunnerError`
/// surfaced by [`overdrive_worker::probe_runner::ProbeRunner::probe`]
/// so the composition root can convert via `#[from]` rather than
/// stringifying through [`ControlPlaneError::Internal`] (which
/// would destroy the structured variant fidelity per
/// `.claude/rules/development.md` § "Distinct failure modes get
/// distinct error variants").
#[derive(Debug, Error)]
pub enum ProbeRunnerBootError {
    /// The sacrificial-loopback probe configured on the
    /// `ProbeRunner` did not return Pass. Indicates the TCP
    /// adapter is wired but unable to complete a basic round-trip
    /// against `127.0.0.1` — typically because (a) the loopback
    /// interface is down, (b) the sim adapter was given a
    /// pre-enqueued Fail outcome (acceptance-test injection), or
    /// (c) a probe-adapter regression has broken the connect path.
    #[error(
        "ProbeRunner Earned-Trust probe failed — the sacrificial \
         loopback probe configured at composition root did not \
         return Pass. The TCP probe adapter is wired but cannot \
         complete a round-trip against 127.0.0.1; subsequent probe \
         dispatches would silently misclassify every workload.\n\
         \n\
         Try:\n\
           - `ip link show lo` to verify the loopback interface is up.\n\
           - `ss -tlnp` to inspect listening sockets on this host.\n\
         \n\
         Underlying: {source}"
    )]
    Probe {
        /// Underlying `ProbeRunnerError` from
        /// `ProbeRunner::probe`.
        #[source]
        source: overdrive_worker::probe_runner::ProbeRunnerError,
    },
}

impl ControlPlaneError {
    /// Construct an [`ControlPlaneError::Internal`] from a context label
    /// and an underlying error. The rendered message is
    /// `"{context}: {source}"`, matching the shape call sites previously
    /// built by hand with `format!`.
    ///
    /// Using this constructor over raw `Internal(format!(...))` keeps
    /// the 40-odd infrastructure error sites in this crate consistent
    /// and lets a future `Internal` variant evolution (e.g. structured
    /// `{context, source}`) land without touching every call site.
    pub fn internal(context: impl fmt::Display, source: impl fmt::Display) -> Self {
        Self::Internal(format!("{context}: {source}"))
    }
}

/// Map a `ControlPlaneError` to `(StatusCode, ErrorBody)` per ADR-0015
/// Table §3. Exhaustive at the enum level so a forgotten variant is a
/// compile-time error.
///
/// Returns the body as a plain struct (not `Json<...>`) so callers can
/// decide whether to serialise immediately or attach headers first;
/// [`IntoResponse`] wraps this in `Json(...)` for the axum handler path.
#[must_use]
// Exhaustive match over every `ControlPlaneError` variant — the
// exhaustiveness IS the contract (a forgotten variant is a compile
// error). The arm count crossed the 100-line ceiling as boot-time and
// probe-runner variants accumulated; splitting the match would trade the
// single-glance exhaustiveness guarantee for an arbitrary cut. Same
// disposition as `run_server_with_obs_and_driver`.
#[allow(clippy::too_many_lines)]
pub fn to_response(err: ControlPlaneError) -> (StatusCode, ErrorBody) {
    use overdrive_core::aggregate::AggregateError;
    use overdrive_core::traits::intent_store::IntentStoreError;

    match err {
        ControlPlaneError::Validation { message, field } => {
            (StatusCode::BAD_REQUEST, ErrorBody { error: "validation".into(), message, field })
        }
        ControlPlaneError::NotFound { resource } => (
            StatusCode::NOT_FOUND,
            ErrorBody { error: "not_found".into(), message: resource, field: None },
        ),
        ControlPlaneError::Conflict { message } => {
            (StatusCode::CONFLICT, ErrorBody { error: "conflict".into(), message, field: None })
        }
        ControlPlaneError::Intent(IntentStoreError::NotFound) => (
            StatusCode::NOT_FOUND,
            ErrorBody {
                error: "not_found".into(),
                message: "intent-store key not found".into(),
                field: None,
            },
        ),
        ControlPlaneError::Intent(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Observation(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Aggregate(e) => {
            // Pull the offending field out of the wrapped `AggregateError`
            // when available, so the ErrorBody's `field` is not always
            // `None` for validation errors routed through `#[from]`.
            let field = match &e {
                AggregateError::Validation { field, .. } => Some((*field).to_string()),
                AggregateError::Id(_) | AggregateError::Resources(_) => None,
            };
            (
                StatusCode::BAD_REQUEST,
                ErrorBody { error: "validation".into(), message: e.to_string(), field },
            )
        }
        ControlPlaneError::Tls(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Cgroup(e) => (
            // Pre-flight refusal happens BEFORE any listener binds; this
            // arm exists for completeness so the enum match stays
            // exhaustive. In practice a Cgroup error never reaches an
            // HTTP response — the operator sees it on stderr at boot.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::CgroupBootstrap(e) => (
            // Same shape as `Cgroup` above: control-plane slice
            // bootstrap failures happen BEFORE the listener binds.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::WorkloadsBootstrap(e) => (
            // Same shape as `CgroupBootstrap` above: workloads-slice
            // bootstrap failures happen BEFORE the listener binds.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::NodeHealthWrite(e) => (
            // Same shape as `WorkloadsBootstrap` above: boot-time
            // node_health writes happen BEFORE the listener binds, so
            // this arm is exhaustiveness-only. The composition root
            // branches on `matches!(e, NodeHealthWrite(_))` for
            // structured boot diagnostics.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::ViewStoreBoot(e) => (
            // Same shape as `Cgroup` above: ViewStore boot failures
            // happen BEFORE the listener binds, so this arm is
            // exhaustiveness-only. The composition root branches on
            // the typed variant (`matches!(e, ViewStoreBoot(_))`) to
            // emit `health.startup.refused`.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::DataplaneBoot(e) => (
            // Same shape as `ViewStoreBoot` above: dataplane boot
            // failures (Construct / Probe / IfaceAddrResolution)
            // happen BEFORE the listener binds. The composition
            // root branches on the typed variant
            // (`matches!(e, DataplaneBoot(_))`) to emit
            // `health.startup.refused` per architecture.md § 5.3;
            // this arm exists only for enum exhaustiveness.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::MtlsBoot(e) => (
            // Same boot-path shape as `DataplaneBoot` above: the
            // transparent-mTLS layer is wired + probed AFTER
            // `IdentityMgr` and BEFORE the listener binds (D-MTLS-17),
            // so this arm is exhaustiveness-only. The composition root
            // branches on the typed variant
            // (`matches!(e, ControlPlaneError::MtlsBoot(_))`) to emit
            // `health.startup.refused` and refuse to boot fail-closed;
            // this arm exists only for enum exhaustiveness.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::VipAllocatorConfig(e) => (
            // Same shape as `ViewStoreBoot` above: VIP-allocator
            // config refusals happen BEFORE the listener binds. The
            // parser at `vip_allocator_config::parse_vip_allocator_section`
            // emits `health.startup.refused` itself; this arm exists
            // only for enum exhaustiveness.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::VipAllocator(e) => {
            // Branch on the typed inner cause per service-vip-allocator
            // step 02-03d. Pool exhaustion is operator-actionable (provision
            // a larger range or release stale allocations) — HTTP 503 with a
            // discrete `error = "pool_exhausted"` discriminator the CLI can
            // branch on without `Display`-grepping. Other inner causes
            // (Storage I/O, Envelope corruption) are infra failures and
            // fall through to HTTP 500.
            use overdrive_dataplane::allocators::{
                PersistentAllocatorError, ServiceVipAllocatorError,
            };
            match e {
                PersistentAllocatorError::Allocator(ServiceVipAllocatorError::Exhausted {
                    allocated,
                    capacity,
                }) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorBody {
                        error: "pool_exhausted".into(),
                        message: format!(
                            "service VIP pool exhausted: allocated {allocated} of {capacity}",
                        ),
                        field: None,
                    },
                ),
                other => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorBody { error: "internal".into(), message: other.to_string(), field: None },
                ),
            }
        }
        ControlPlaneError::ProbeRunnerBoot(e) => (
            // Same shape as `DataplaneBoot` above: probe-gate
            // failures happen BEFORE the listener binds. The
            // composition root branches on the typed variant
            // (`matches!(e, ProbeRunnerBoot(_))`) to emit
            // `health.startup.refused` per ADR-0054 § 7; this arm
            // exists only for enum exhaustiveness.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::ListenerFactRebuild(source) => (
            // Same shape as `ViewStoreBoot` / `DataplaneBoot` above: the
            // boot-time listener-fact rebuild happens BEFORE the listener
            // binds, so this arm is exhaustiveness-only. The composition
            // root branches on the typed variant
            // (`matches!(e, ListenerFactRebuild(_))`) for structured
            // startup diagnostics. The boxed `ConvergenceError`'s own
            // `Display` carries the underlying cause.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: source.to_string(), field: None },
        ),
        err @ ControlPlaneError::ServiceVipMissing { .. } => (
            // ADR-0064 § 4 / OQ-4: a persisted Service with no allocator
            // entry is an internal-invariant violation (the
            // submit-time-allocate / boot-rebuild contract was broken),
            // NOT operator-actionable input — so it maps to HTTP 500
            // `internal`, never a 4xx. `err.to_string()` carries the
            // `spec_digest` and the named invariant into `body.message`
            // for diagnostics, matching the `Tls(e)` / `CgroupBootstrap(e)`
            // precedent above.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: err.to_string(), field: None },
        ),
        ControlPlaneError::Ca(e) => (
            // Same shape as `ViewStoreBoot` / `DataplaneBoot` above: the
            // ephemeral workload-CA composition (ADR-0067 D3 rev 4) happens
            // BEFORE the listener binds, so this arm is exhaustiveness-only.
            // The composition root branches on the typed variant
            // (`matches!(e, ControlPlaneError::Ca(_))`) for structured startup
            // diagnostics; the typed `CaError`'s own `Display` carries the
            // signing / subject / adoption cause.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::CaBoot(e) => (
            // Same shape as `Ca` / `ViewStoreBoot` / `DataplaneBoot` above: the
            // persistent built-in-CA boot (built-in-ca-operator-composition
            // D-OC-5 / #215) happens BEFORE the listener binds, so this arm is
            // exhaustiveness-only. The composition root branches on the typed
            // variant (`matches!(e, ControlPlaneError::CaBoot(_))`) for
            // structured startup diagnostics; the typed `CaBootError`'s own
            // `Display` carries the distinct boot cause (absent-KEK vs
            // wrong-KEK vs tampered-envelope).
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Internal(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: msg, field: None },
        ),
    }
}

impl IntoResponse for ControlPlaneError {
    fn into_response(self) -> Response {
        let (status, body) = to_response(self);
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0064 § 4 / OQ-4: a persisted-and-describable Service ALWAYS
    /// has an allocated VIP (submit-time admission allocates before the
    /// intent is written; the boot rebuild re-seeds the allocator memo
    /// from the intent SSOT — ADR-0049 § 4 / § 8). A `None` from the
    /// read-only `allocator.get(&spec_digest)` at describe time is
    /// therefore an INTERNAL-INVARIANT violation, not a client-visible
    /// 4xx — it means the allocate-or-rebuild invariant was broken. It
    /// maps to HTTP 500 with the `internal` error kind, and the rendered
    /// message names the `spec_digest` so an operator can trace which
    /// Service lost its allocator memo.
    ///
    /// `ServiceVipMissing` is a DEDICATED variant per
    /// `.claude/rules/development.md` § "Errors → Never flatten a typed
    /// error to `Internal(String)`" — NOT `ControlPlaneError::internal(...)`.
    #[test]
    fn service_vip_missing_maps_to_http_500() {
        let spec_digest =
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_owned();

        let err = ControlPlaneError::ServiceVipMissing { spec_digest: spec_digest.clone() };

        let (status, body) = to_response(err);

        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a missing allocator entry is an internal-invariant violation → HTTP 500, not a 4xx",
        );
        assert_eq!(body.error, "internal");
        assert!(body.field.is_none(), "ServiceVipMissing is not a field-level validation error");
        assert!(
            body.message.contains(&spec_digest),
            "the rendered message must name the spec_digest for diagnostics; got {:?}",
            body.message,
        );
    }
}
