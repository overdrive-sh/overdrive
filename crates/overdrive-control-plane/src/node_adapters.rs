//! The driven adapters `run_server` composes the node over (GH #295,
//! NO-OVERRIDE-SEAMS full scope and STORES-IN, ADR-0219; feature-delta
//! § *Composition*, "Driven adapters of `run_server`").
//!
//! The composition root (`overdrive serve`, or a test) builds every driven
//! adapter and passes it in; neither `run_server` nor `boot_server`
//! constructs one, and `ServerConfig` carries configuration only. Every field
//! of every bundle is required: no `Default`, no builder
//! (`development.md` § "Port-trait dependencies").

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::ca::kek::Kek;
use overdrive_core::traits::ca::CaFactory;
use overdrive_core::traits::cgroup_accounting::CgroupAccounting;
use overdrive_core::traits::cgroup_fs::CgroupFs;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::mtls_enforcement::MtlsEnforcementFactory;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{HttpProber, TcpProber};
use overdrive_core::traits::transport::Transport;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::beacon::BeaconIo;
use overdrive_dataplane::allocators::VipRange;
use overdrive_host::ca::RootKeyAeadCodec;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_port::MtlsIntercept;

use crate::dataplane_config::DataplaneConfig;
use crate::dns_responder::GuestDnsFactory;
use crate::error::{self, ControlPlaneError};
use crate::guest_network::flow_io::GuestFlowIo;
use crate::journal::JournalStore;
use crate::view_store::ViewStore;
use crate::{ServerConfig, cgroup_preflight, observation_wiring};

/// The driven adapters `run_server` composes over. Every field required;
/// no `Default`, no builder (`development.md` § "Port-trait dependencies").
pub struct NodeAdapters {
    /// The adapters both boot paths compose over (`run_server` and the
    /// supplied-owner path).
    pub boot: BootAdapters,
    /// The cgroupfs adapter: the workloads-slice bootstrap, the
    /// `CgroupManager` and the VM driver use it. `overdrive serve` passes the
    /// handle its ADR-0054 probe accepted (the probed handle is the used
    /// handle).
    pub cgroup_fs: Arc<dyn CgroupFs>,
    /// The probe runner's probers.
    pub probers: ProbeAdapters,
    /// The VM driver's driven ports and the node's VM capability statement.
    pub vm: VmAdapters,
    /// The guest-flow owner's seam.
    pub guest_flow: GuestFlowAdapter,
}

/// The adapters `boot_server` composes over, on both boot paths.
pub struct BootAdapters {
    /// The one clock of the boot: convergence loop, `TickContext`, probe
    /// runner, exec gate, VM driver, workflow engine, node-health writer,
    /// streaming handlers, supervisor. `SystemClock` in production.
    pub clock: Arc<dyn Clock>,
    /// The workflow engine's RNG, the workload-identity CA's key generation
    /// and the address pool's launch tokens. `OsEntropy` in production.
    pub entropy: Arc<dyn Entropy>,
    /// The workflow engine's `ctx.call` transport. `TcpTransport` in
    /// production.
    pub transport: Arc<dyn Transport>,
    /// The workload-identity CA's KEK (ADR-0063). `SystemdCredsKeyring` in
    /// production.
    pub kek: Arc<dyn Kek>,
    /// The observation store. In production the `LocalObservationStore`
    /// `observation_wiring::wire_single_node_observation(&config.data_dir)`
    /// opens.
    pub observation: Arc<dyn ObservationStore>,
    /// The VM host state boot reclamation reads (composed whether or not a
    /// `Vm` driver is). `RealVmHostState` over the cgroup root,
    /// `/run/overdrive/vm` and `clone_index_dir(&config.data_dir)` in
    /// production.
    pub vm_host_state: Arc<dyn VmHostState>,
    /// The node's persistent stores (STORES-IN). In production
    /// `StoreAdapters::open(config)`.
    pub stores: StoreAdapters,
    /// The workload-identity CA's factory and codec (STORES-IN). In
    /// production `CaAdapters::host()`. The boot builds the CA over this
    /// bundle's `entropy`, then runs its generate-or-adopt.
    pub ca: CaAdapters,
    /// Boot step 3's dataplane block.
    pub dataplane: Arc<dyn DataplaneComposer>,
    /// The transparent-mTLS adapters.
    pub mtls: MtlsAdapters,
    /// ADR-0138. `HostGuestDnsFactory` in production.
    pub guest_dns: Arc<dyn GuestDnsFactory>,
}

/// The transparent-mTLS adapters (ADR-0138, ADR-0219).
pub struct MtlsAdapters {
    /// `HostMtlsIntercept` in production.
    pub intercept: Arc<dyn MtlsIntercept>,
    /// `HostMtlsEnforcementFactory` in production.
    pub enforcement: Arc<dyn MtlsEnforcementFactory>,
}

/// The node's persistent stores (STORES-IN, ADR-0219). Opened by the
/// composition root over `config.data_dir`; `boot_server` opens nothing.
/// Not `Clone`: one `StoreAdapters` serves one boot. Every boot opens its
/// stores from disk (`StoreAdapters::open` or `NodeAdapters::host`), so a
/// restart or crash test reopens the files the previous server released and
/// reruns `LocalIntentStore::open`'s boot-validation walks, as a new process
/// does. `views` and `journal` are public `Arc` fields, so a caller could
/// build a second boot's `StoreAdapters` from the first boot's `Arc`s by
/// struct literal; that is a caller error, and every restart and crash test
/// reopens through `StoreAdapters::open` or `NodeAdapters::host`.
pub struct StoreAdapters {
    /// The intent store and the file it was opened from.
    pub intent: IntentStoreAdapter,
    /// The reconciler view store (ADR-0035). `RedbViewStore` over
    /// `<data_dir>/reconcilers/memory.redb` in production; `SimViewStore`
    /// in a simulated test. The boot probes and bulk-loads it in each
    /// `runtime.register`.
    pub views: Arc<dyn ViewStore>,
    /// The workflow journal (ADR-0066). `RedbJournalStore` over
    /// `<data_dir>/workflow-journal.redb` in production; `SimJournalStore`
    /// in a simulated test.
    pub journal: Arc<dyn JournalStore>,
}

impl StoreAdapters {
    /// The host stores over `config.data_dir`, in this order: the intent
    /// store (`IntentStoreAdapter::open`, which creates `data_dir` when
    /// missing), the view store (`RedbViewStore::open(&config.data_dir)`),
    /// the journal (`redb::Database::create(<data_dir>/workflow-journal.redb)`
    /// then `RedbJournalStore::new`).
    ///
    /// # Errors
    ///
    /// The first failure in that order: the intent open's
    /// `ControlPlaneError::Internal` ("open LocalIntentStore"),
    /// `ControlPlaneError::ViewStoreBoot(ViewStoreBootError::Open { path,
    /// source })`, the journal create's `ControlPlaneError::Internal`
    /// ("create workflow-journal redb"). A file opened before the failure is
    /// closed by drop.
    pub fn open(config: &ServerConfig) -> Result<Self, ControlPlaneError> {
        let intent = IntentStoreAdapter::open(&config.data_dir)?;
        let views: Arc<dyn ViewStore> =
            Arc::new(crate::view_store::redb::RedbViewStore::open(&config.data_dir).map_err(
                |e| error::ViewStoreBootError::Open {
                    path: crate::view_store::redb::RedbViewStore::resolve_path(&config.data_dir),
                    source: e,
                },
            )?);
        let journal_path = config.data_dir.join("workflow-journal.redb");
        let journal_db = Arc::new(
            redb::Database::create(&journal_path)
                .map_err(|e| ControlPlaneError::internal("create workflow-journal redb", e))?,
        );
        let journal: Arc<dyn JournalStore> =
            Arc::new(crate::journal::RedbJournalStore::new(journal_db));
        Ok(Self { intent, views, journal })
    }
}

/// The node's intent store (Phase-1 single mode: `LocalIntentStore` over
/// redb) and the path of its file. The path travels with the store, so the
/// CA boot's refuse-to-start remediations, the listener-fact rebuild and
/// `HydrationContext::intent_redb_path` name the file that was opened. Not
/// `Clone`, for the reason `StoreAdapters` is not; the boot shares the store
/// through `store().clone()` of the `Arc`.
pub struct IntentStoreAdapter {
    store: Arc<LocalIntentStore>,
    path: PathBuf,
}

impl IntentStoreAdapter {
    /// `LocalIntentStore::open(<data_dir>/intent.redb)`; creates `data_dir`
    /// when missing.
    ///
    /// # Errors
    ///
    /// `ControlPlaneError::Internal` ("open LocalIntentStore").
    pub fn open(data_dir: &Path) -> Result<Self, ControlPlaneError> {
        let path = data_dir.join("intent.redb");
        let store = Arc::new(
            LocalIntentStore::open(&path)
                .map_err(|e| ControlPlaneError::internal("open LocalIntentStore", e))?,
        );
        Ok(Self { store, path })
    }

    /// The opened store.
    #[must_use]
    pub const fn store(&self) -> &Arc<LocalIntentStore> {
        &self.store
    }

    /// The file `open` opened.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The workload-identity CA's factory and the codec that seals its root key
/// under the KEK (ADR-0063, STORES-IN). The boot calls `factory.build` once,
/// over `BootAdapters.entropy`, so the CA's key generation always draws from
/// the boot's one entropy: a test that replaces `entropy` by struct update
/// cannot leave a CA keyed to another source. The built CA holds no root;
/// the boot's `ca_boot::boot_ca` / `bootstrap_node_intermediate` generate or
/// adopt the persistent root and the node intermediate. Each boot builds its
/// own CA.
pub struct CaAdapters {
    /// `RcgenCaFactory` in production; `SimCaFactory` or a test-local
    /// factory in a test.
    pub factory: Arc<dyn CaFactory>,
    /// `RootKeyAeadCodec` (the only codec; `boot_ca` takes it concretely).
    pub root_key_codec: Arc<RootKeyAeadCodec>,
}

impl CaAdapters {
    /// `RcgenCaFactory` over the workload CA trust-domain subject
    /// `spiffe://overdrive.local/overdrive/ca`, and `RootKeyAeadCodec::new()`.
    /// Infallible; no I/O.
    #[must_use]
    pub fn host() -> Self {
        let ca_subject = overdrive_core::SpiffeId::new("spiffe://overdrive.local/overdrive/ca")
            .unwrap_or_else(|e| unreachable!("CA trust-domain subject is a valid SPIFFE URI: {e}"));
        Self {
            factory: Arc::new(overdrive_host::ca::RcgenCaFactory::new(ca_subject)),
            root_key_codec: Arc::new(RootKeyAeadCodec::new()),
        }
    }
}

/// The probe runner's probers (ADR-0054 § 7). `TokioTcpProber` and
/// `HyperHttpProber` in production.
pub struct ProbeAdapters {
    /// The TCP prober.
    pub tcp: Arc<dyn TcpProber>,
    /// The HTTP prober.
    pub http: Arc<dyn HttpProber>,
}

/// The VM driver's driven ports (DG-4), the cgroup accounting probed beside
/// the VMM (ADR-0082 § D8 "Composition") and the composition root's
/// statement of whether this node requires VM capability. A simulated VMM
/// with a real beacon (or the reverse) is a caller error: the two travel
/// together (DG-4).
pub struct VmAdapters {
    /// `CloudHypervisorVmm` in production; `SimVmm` in a simulated-VM test.
    pub vmm: Arc<dyn Vmm>,
    /// `VsockBeaconIo` in production; `SimBeaconIo` with `SimVmm`.
    pub beacon: Arc<dyn BeaconIo>,
    /// `RealCgroupAccounting` in production.
    pub cgroup_accounting: Arc<dyn CgroupAccounting>,
    /// What a failed VM probe means at boot step 2.
    pub capability: VmCapability,
}

/// What a failed `Vmm::probe` or `CgroupAccounting::probe` (or the
/// `/dev/kvm` group read or the clone-staging setup after them) means at
/// boot step 2. The probes run unconditionally either way (Earned Trust).
/// Why `Required` exists although `overdrive serve` never passes it:
/// ADR-0219, Decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmCapability {
    /// The node boots without a `Vm` driver and logs
    /// `driver.vm.not_composed`; a `[vm]` deploy fails at dispatch.
    /// `overdrive serve` passes this.
    Optional,
    /// The boot refuses: `health.startup.refused`, reason `vmm.probe`,
    /// `ControlPlaneError::VmmBoot`. A root whose VMM is a double it
    /// constructed passes this.
    Required,
}

impl VmCapability {
    /// `"optional"` | `"required"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Optional => "optional",
            Self::Required => "required",
        }
    }
}

/// The guest-flow owner's seam (`GuestFlowIo`, crate-private). Opaque:
/// outside the crate only the host seam can be named, so every out-of-crate
/// `serve` runs the production listeners and forwarder (PORTS-CONFIG).
pub struct GuestFlowAdapter(pub(crate) Arc<dyn GuestFlowIo>);

impl GuestFlowAdapter {
    /// `HostGuestFlowIo::new()`. Infallible; touches no kernel state.
    #[must_use]
    pub fn host() -> Self {
        Self(Arc::new(crate::guest_network::flow_io::HostGuestFlowIo::new()))
    }
}

impl NodeAdapters {
    /// The host bundle — the composition `overdrive serve` runs — over the
    /// configuration it will boot (every path is derived from
    /// `config.data_dir`), with the two adapters the binary boundary chooses
    /// passed in: `cgroup_fs` (the handle its ADR-0054 probe already
    /// accepted) and `kek` (`SystemdCredsKeyring` in `serve`). Every other
    /// field is the host adapter named in its docstring, `capability` is
    /// `VmCapability::Optional` and `guest_flow` is `GuestFlowAdapter::host()`.
    /// Order (STORES-IN): the infallible adapters, then
    /// `StoreAdapters::open(config)`, then the observation store; `ca` is
    /// `CaAdapters::host()` (the boot builds the CA over `boot.entropy`).
    ///
    /// Precondition: nothing in this process still holds the same
    /// `data_dir`'s redb files.
    ///
    /// # Errors
    ///
    /// The first failing open (`StoreAdapters::open`'s three, then the
    /// observation store's).
    pub fn host(
        config: &ServerConfig,
        cgroup_fs: Arc<dyn CgroupFs>,
        kek: Arc<dyn Kek>,
    ) -> Result<Self, ControlPlaneError> {
        let clock: Arc<dyn Clock> = Arc::new(overdrive_host::SystemClock);
        let entropy: Arc<dyn Entropy> = Arc::new(overdrive_host::OsEntropy);
        let transport: Arc<dyn Transport> = Arc::new(overdrive_host::TcpTransport::default());
        let probers = ProbeAdapters {
            tcp: Arc::new(overdrive_worker::probe_runner::TokioTcpProber::new()),
            http: Arc::new(overdrive_worker::probe_runner::HyperHttpProber::new()),
        };
        let vm = VmAdapters {
            vmm: Arc::new(overdrive_host::CloudHypervisorVmm::new()),
            beacon: Arc::new(overdrive_host::VsockBeaconIo::new()),
            cgroup_accounting: Arc::new(overdrive_host::RealCgroupAccounting::new()),
            capability: VmCapability::Optional,
        };
        let dataplane: Arc<dyn DataplaneComposer> = Arc::new(HostDataplaneComposer::new());
        let mtls = MtlsAdapters {
            intercept: Arc::new(overdrive_worker::mtls_intercept_port::HostMtlsIntercept::new()),
            enforcement: Arc::new(overdrive_dataplane::mtls::HostMtlsEnforcementFactory::new()),
        };
        let guest_dns: Arc<dyn GuestDnsFactory> =
            Arc::new(crate::dns_responder::HostGuestDnsFactory);
        let vm_host_state: Arc<dyn VmHostState> = Arc::new(overdrive_host::RealVmHostState::new(
            PathBuf::from(cgroup_preflight::DEFAULT_CGROUP_ROOT),
            PathBuf::from("/run/overdrive/vm"),
            overdrive_core::vm::config::clone_index_dir(&config.data_dir),
        ));
        let stores = StoreAdapters::open(config)?;
        let observation: Arc<dyn ObservationStore> =
            Arc::from(observation_wiring::wire_single_node_observation(&config.data_dir)?);
        let ca = CaAdapters::host();
        Ok(Self {
            boot: BootAdapters {
                clock,
                entropy,
                transport,
                kek,
                observation,
                vm_host_state,
                stores,
                ca,
                dataplane,
                mtls,
                guest_dns,
            },
            cgroup_fs,
            probers,
            vm,
            guest_flow: GuestFlowAdapter::host(),
        })
    }
}

// ---------------------------------------------------------------------------
// Dataplane composer
// ---------------------------------------------------------------------------

/// Boot step 3's dataplane block (ADR-0052, ADR-0061: provision → construct
/// → probe), behind a port because its production construction has host
/// effects that must follow the boot's startup checks.
#[async_trait::async_trait]
pub trait DataplaneComposer: Send + Sync {
    /// Called once per boot, at step 3.
    ///
    /// Ok: a dataplane that passed its Earned-Trust probe, and the node's
    /// address on `plan.config.client_iface`.
    ///
    /// Err: the `ControlPlaneError` the boot returns for that failure —
    /// `DataplaneBoot(Provision | Construct | Probe)`, or the client
    /// interface's address-resolution error. Nothing the call created
    /// survives an Err (a constructed dataplane is dropped, which detaches
    /// its programs and unpins its maps).
    async fn compose(
        &self,
        plan: DataplanePlan<'_>,
    ) -> Result<ComposedDataplane, ControlPlaneError>;
}

/// What the boot hands the composer.
pub struct DataplanePlan<'a> {
    /// The `[dataplane]` section.
    pub config: &'a DataplaneConfig,
    /// The configured service VIP range.
    pub vip_range: &'a VipRange,
    /// `config.dataplane_pin_dir`, or `overdrive_dataplane::DEFAULT_PIN_DIR`.
    pub pin_dir: &'a Path,
    /// `config.dataplane_cgroup_attach_path`, or
    /// `overdrive_dataplane::DEFAULT_CGROUP_ATTACH_PATH`.
    pub cgroup_attach_path: &'a Path,
}

/// What the composer returns.
pub struct ComposedDataplane {
    /// A dataplane that passed its probe.
    pub dataplane: Arc<dyn Dataplane>,
    /// The node's address on the client interface.
    pub host_ipv4: Ipv4Addr,
}

/// Production: provisions the veth pair when the configured interfaces are
/// the default names (ADR-0061 § 1), constructs `EbpfDataplane` with
/// `plan.pin_dir` and `plan.cgroup_attach_path`, runs its probe, then
/// resolves `host_ipv4` from `plan.config.client_iface`. Infallible
/// constructor; touches nothing until `compose`. On a probe `Err` the
/// constructed `EbpfDataplane` is dropped before `compose` returns.
pub struct HostDataplaneComposer {
    #[cfg(feature = "integration-tests")]
    probe_fault: parking_lot::Mutex<Option<overdrive_core::traits::dataplane::DataplaneError>>,
    #[cfg(not(feature = "integration-tests"))]
    _private: (),
}

impl HostDataplaneComposer {
    /// The production composer.
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "the composition root names each adapter explicitly; no `Default`"
    )]
    pub const fn new() -> Self {
        Self {
            #[cfg(feature = "integration-tests")]
            probe_fault: parking_lot::const_mutex(None),
            #[cfg(not(feature = "integration-tests"))]
            _private: (),
        }
    }

    /// `integration-tests` only. The production composer, except that
    /// `compose` arms `fault` on the `EbpfDataplane` it constructs
    /// (`set_probe_fault`) before running its probe, so `compose` takes its
    /// real `Err` path: `DataplaneBoot(Probe { source: fault })`, the
    /// dataplane dropped.
    #[cfg(feature = "integration-tests")]
    #[must_use]
    pub fn with_probe_fault(fault: overdrive_core::traits::dataplane::DataplaneError) -> Self {
        Self { probe_fault: parking_lot::Mutex::new(Some(fault)) }
    }
}

#[async_trait::async_trait]
impl DataplaneComposer for HostDataplaneComposer {
    async fn compose(
        &self,
        plan: DataplanePlan<'_>,
    ) -> Result<ComposedDataplane, ControlPlaneError> {
        let dataplane_cfg = plan.config;
        // ADR-0061 § 3: the single-node serve boot provisions the host-netns
        // veth pair BEFORE `EbpfDataplane::new` — only when the configured
        // ifaces are the DEFAULT veth names (ADR-0061 § 1); real NICs are
        // never provisioned.
        let is_default_veth = dataplane_cfg.client_iface
            == crate::veth_provisioner::DEFAULT_CLIENT_IFACE
            && dataplane_cfg.backend_iface == crate::veth_provisioner::DEFAULT_BACKEND_IFACE;
        if is_default_veth {
            let veth_plan = crate::veth_provisioner::derive_veth_plan(
                &dataplane_cfg.client_iface,
                &dataplane_cfg.backend_iface,
                plan.vip_range.first_range(),
            );
            if let Err(source) = crate::veth_provisioner::provision(&veth_plan).await {
                tracing::warn!(
                    name: "health.startup.refused",
                    reason = "dataplane.provision",
                    client_iface = %dataplane_cfg.client_iface,
                    backend_iface = %dataplane_cfg.backend_iface,
                    error = %source,
                    "single-node veth provisioning failed; refusing to boot"
                );
                return Err(ControlPlaneError::DataplaneBoot(
                    error::DataplaneBootError::Provision { source },
                ));
            }
        }

        #[allow(unused_mut, reason = "mutated only under the integration-tests probe fault")]
        let mut ebpf_dataplane = overdrive_dataplane::EbpfDataplane::new_with_pin_dir(
            &dataplane_cfg.client_iface,
            &dataplane_cfg.backend_iface,
            plan.pin_dir,
            plan.cgroup_attach_path,
        )
        .map_err(|source| error::DataplaneBootError::Construct {
            client_iface: dataplane_cfg.client_iface.clone(),
            backend_iface: dataplane_cfg.backend_iface.clone(),
            source,
        })?;

        #[cfg(feature = "integration-tests")]
        {
            // `parking_lot` has no poisoning, so the armed fault is never
            // silently dropped into the success path.
            let fault = self.probe_fault.lock().take();
            if let Some(fault) = fault {
                ebpf_dataplane.set_probe_fault(fault);
            }
        }

        // Earned-Trust probe (architecture.md § 5.4): wire → probe → use. On
        // failure the `return` drops `ebpf_dataplane`, which detaches XDP and
        // unlinks the SERVICE_MAP pin (`EbpfDataplane::Drop`).
        if let Err(source) = ebpf_dataplane.probe().await {
            tracing::warn!(
                name: "health.startup.refused",
                reason = "dataplane.probe",
                client_iface = %dataplane_cfg.client_iface,
                backend_iface = %dataplane_cfg.backend_iface,
                error = %source,
                "Earned-Trust probe failed; refusing to boot"
            );
            return Err(ControlPlaneError::DataplaneBoot(error::DataplaneBootError::Probe {
                source,
            }));
        }

        let host_ipv4 =
            crate::iface::resolve_iface_ipv4(&dataplane_cfg.client_iface).map_err(|source| {
                error::DataplaneBootError::IfaceAddrResolution {
                    iface: dataplane_cfg.client_iface.clone(),
                    source,
                }
            })?;
        Ok(ComposedDataplane { dataplane: Arc::new(ebpf_dataplane), host_ipv4 })
    }
}
