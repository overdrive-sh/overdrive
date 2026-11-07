//! Gateway Application owner, composition surface and admission state.
//!
//! The owner (`GatewayApplicationOwner`) is the only mutator of the staged /
//! current / draining generations, the listener and the Application status
//! row. The admission refusal arms, the per-connection and per-request leases
//! and the accept loop belong to later steps and stay scaffolded.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZeroU32;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::id::NodeId;
use overdrive_core::public_ingress::{
    CertifiedKeyProvenance, GatewayApplicationGenerationId, GatewayApplicationGenerationStatus,
    GatewayApplicationStatusRowV1, GatewayApplicationUnavailableCause, GatewayConnectPathStatus,
    GatewayIdentityStatus, GatewayListenerStatus, PublicCertifiedKeyStatusRowV1, Route, ServiceKey,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentSubscriptionEvent;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStoreError, ObservationWrite,
};
use overdrive_core::traits::{IntentStore, ObservationStore, ServiceVipView};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::certified_key::{
    InstallPublicCertifiedKey, ManualCertifiedKeySource, PublicCertifiedKeyAvailability,
    PublicCertifiedKeyCustody, PublicCertifiedKeyCustodyHandle, PublicCertifiedKeyError,
    PublicCertifiedKeySnapshot,
};
use crate::connector::{GatewayUpstreamConnectError, GatewayUpstreamConnector};
use crate::frontend::IntentServiceFrontendResolver;
use crate::ports::{
    GatewayClientMtls, GatewayConnectCleanup, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewayIdentityLifecycleControl,
    GatewayIdentityLifecycleError, GatewayRelay, GatewayUpstreamSeal, ServiceFrontendResolve,
    ServiceFrontendResolveError,
};
use crate::route_set::{
    PublicRouteSetError, PublicRouteSetHandle, PublicRouteSetOwner, ROUTE_SET_KEY,
};
use crate::runtime::GatewayLimits;
use crate::runtime::exchange::{ConnectionServices, DrainSignal, serve_connection};
use crate::{GatewayConfig, GatewayConfigError};

#[cfg(all(test, feature = "integration-tests"))]
mod held_probe_doubles;
#[cfg(test)]
mod ledger_acceptance;
#[cfg(all(test, feature = "integration-tests"))]
mod probe_completion_acceptance;
#[cfg(all(test, feature = "integration-tests"))]
mod quint_gateway_application;

/// Depth of the owner's gate-result channel.
const GATE_REPORT_CAPACITY: usize = 8;
/// Depth of the Application command channel: exactly one shutdown close plus
/// one retire command.
const APPLICATION_COMMAND_CAPACITY: usize = 2;
/// Retry period of a failed status write.
const STATUS_RETRY: Duration = Duration::from_secs(1);

/// The wait on the injected clock after a failed `accept(2)`, so descriptor
/// exhaustion does not spin the listener task.
const ACCEPT_FAILURE_BACKOFF: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// Applied generation
// ---------------------------------------------------------------------------

/// The authority that one coherent Gateway Application generation was
/// promoted. Constructible only inside the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayAppliedGeneration {
    application_generation: GatewayApplicationGenerationId,
}

impl GatewayAppliedGeneration {
    pub(crate) const fn from_promotion(
        application_generation: GatewayApplicationGenerationId,
    ) -> Self {
        Self { application_generation }
    }
    pub const fn application_generation(&self) -> GatewayApplicationGenerationId {
        self.application_generation
    }
}

// ---------------------------------------------------------------------------
// Public builder, handle, route-command and status-read ports
// ---------------------------------------------------------------------------

pub struct UnboundGatewayBuilder {
    node_id: NodeId,
    config: GatewayConfig,
    intent: Arc<dyn IntentStore>,
    observations: Arc<dyn ObservationStore>,
    frontend: Arc<dyn ServiceFrontendResolve>,
    clock: Arc<dyn Clock>,
    route_owner: PublicRouteSetOwner,
    route_handle: PublicRouteSetHandle,
    custody: PublicCertifiedKeyCustody,
    custody_handle: PublicCertifiedKeyCustodyHandle,
    manual: ManualCertifiedKeySource,
}

impl UnboundGatewayBuilder {
    /// Reads and validates the manual key first (a refusal mutates nothing),
    /// opens custody and the Route set over the IntentStore, and installs the
    /// initial key. Spawns no task.
    ///
    /// # Errors
    /// `ManualCertifiedKey` for an unreadable or invalid manual key,
    /// `CertifiedKey` for a custody failure and `RouteStore` for a Route-set
    /// store failure.
    pub async fn new(
        node_id: NodeId,
        config: GatewayConfig,
        intent: Arc<dyn IntentStore>,
        observations: Arc<dyn ObservationStore>,
        service_vip_view: Arc<dyn ServiceVipView>,
        key_codec: Arc<PublicCertifiedKeyAeadCodec>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, GatewayBootError> {
        let key_config = config.manual_certified_key();
        let manual = ManualCertifiedKeySource::new(
            key_config.certificate_chain_path().to_path_buf(),
            key_config.private_key_path().to_path_buf(),
        );
        let candidate = manual
            .candidate(UnixInstant::from_clock(clock.as_ref()))
            .await
            .map_err(GatewayBootError::ManualCertifiedKey)?;
        let (mut custody, custody_handle) = PublicCertifiedKeyCustody::open(
            key_config.id().clone(),
            node_id.clone(),
            Arc::clone(&intent),
            Arc::clone(&observations),
            key_codec,
            Arc::clone(&clock),
        )
        .await
        .map_err(GatewayBootError::CertifiedKey)?;
        let (route_owner, route_handle) = PublicRouteSetOwner::open(Arc::clone(&intent))
            .await
            .map_err(GatewayBootError::RouteStore)?;
        let initial = InstallPublicCertifiedKey {
            expected_generation: custody.current_generation(),
            candidate,
            provenance: CertifiedKeyProvenance::Manual,
        };
        custody.install(initial).await.map_err(|error| match error {
            PublicCertifiedKeyError::Source(_) => GatewayBootError::ManualCertifiedKey(error),
            other => GatewayBootError::CertifiedKey(other),
        })?;
        let frontend: Arc<dyn ServiceFrontendResolve> =
            Arc::new(IntentServiceFrontendResolver::new(Arc::clone(&intent), service_vip_view));
        Ok(Self {
            node_id,
            config,
            intent,
            observations,
            frontend,
            clock,
            route_owner,
            route_handle,
            custody,
            custody_handle,
            manual,
        })
    }

    pub fn into_builder(self) -> (GatewayBuilder, GatewayUpstreamSeal) {
        (GatewayBuilder { unbound: self }, GatewayUpstreamSeal::new())
    }
}

pub struct GatewayRuntimeDependencies {
    limits: GatewayLimits,
    dataplane: Arc<dyn GatewayConnectDataplane>,
    client_mtls: Arc<dyn GatewayClientMtls>,
    identity_lifecycle: Arc<dyn GatewayIdentityLifecycleControl>,
    relay: Arc<dyn GatewayRelay>,
}

impl GatewayRuntimeDependencies {
    pub fn new(
        limits: GatewayLimits,
        dataplane: Arc<dyn GatewayConnectDataplane>,
        client_mtls: Arc<dyn GatewayClientMtls>,
        identity_lifecycle: Arc<dyn GatewayIdentityLifecycleControl>,
        relay: Arc<dyn GatewayRelay>,
    ) -> Self {
        Self { limits, dataplane, client_mtls, identity_lifecycle, relay }
    }
}

pub struct GatewayBuilder {
    unbound: UnboundGatewayBuilder,
}

impl GatewayBuilder {
    /// Exactly: build the one budget from `dependencies`' limits,
    /// `GatewayAdmission::new(budget)`, `start_with`.
    pub async fn start(self, dependencies: GatewayRuntimeDependencies) -> StartedGateway {
        let budget = GatewayAdmissionBudget::new(Arc::new(dependencies.limits.clone()));
        let admission = GatewayAdmission::new(budget);
        self.start_with(dependencies, admission).await
    }

    /// Crate-private (DG-10): `start` with an injected admission. The
    /// admission's budget was built from the same limits as `dependencies`.
    pub(crate) async fn start_with(
        self,
        dependencies: GatewayRuntimeDependencies,
        admission: Arc<GatewayAdmission>,
    ) -> StartedGateway {
        let UnboundGatewayBuilder {
            node_id,
            config,
            intent,
            observations,
            frontend,
            clock,
            route_owner,
            route_handle,
            custody,
            custody_handle,
            manual,
        } = self.unbound;
        let GatewayRuntimeDependencies {
            limits,
            dataplane,
            client_mtls,
            identity_lifecycle,
            relay,
        } = dependencies;
        let limits = Arc::new(limits);
        let ledger = GatewayCleanupLedger::new(limits.max_cleanup_ledger_entries());
        let connector = Arc::new(GatewayUpstreamConnector::new(
            Arc::clone(&dataplane),
            client_mtls,
            (*limits).clone(),
            Arc::clone(&clock),
            Arc::clone(admission.budget()),
            Arc::clone(&ledger),
        ));
        let cleanup: Arc<dyn GatewayConnectCleanup> = dataplane.clone();
        let owners = OwnerShutdowns::new();
        let services = ConnectionServices {
            relay,
            connector: Arc::clone(&connector),
            clock: Arc::clone(&clock),
            limits: Arc::clone(&limits),
        };
        let (connections, connections_task, listener, listener_task) =
            Self::spawn_connection_owner(services, &owners);
        let (command_sender, commands) = mpsc::channel(APPLICATION_COMMAND_CAPACITY);
        let owner = GatewayApplicationOwner::fresh(OwnerWiring {
            node_id: node_id.clone(),
            intent,
            certified_key: custody_handle.clone(),
            frontend,
            identity: Arc::clone(&identity_lifecycle),
            connector,
            dataplane,
            admission,
            listener,
            observations: Arc::clone(&observations),
            clock: Arc::clone(&clock),
            limits,
            bind: config.bind(),
            commands,
            shutdown: owners.application.clone(),
        });
        let application = tokio::spawn(owner.run());
        let (manual_source, custody_task, route_set) = Self::spawn_supporting_owners(
            SupportingOwners { manual, custody, route_owner },
            &custody_handle,
            &clock,
            &owners,
        );

        let handle = GatewayHandle {
            tasks: GatewayTaskTree {
                listener: Some(listener_task),
                connections: Some(connections_task),
                application,
                manual_source,
                custody: custody_task,
                route_set,
            },
            shutdown: GatewayShutdownCapabilities {
                application: GatewayApplicationHandle {
                    commands: command_sender,
                    clock: Arc::clone(&clock),
                },
                connections,
                cleanup,
                cleanup_ledger: ledger,
                identity: identity_lifecycle,
                clock,
                owners,
            },
        };
        StartedGateway {
            handle,
            routes: GatewayRouteCommands { routes: route_handle },
            status: GatewayStatusReads {
                observations,
                node_id,
                certified_key_id: custody_handle.id().clone(),
            },
        }
    }

    /// The connections owner task with its shutdown capabilities, and the
    /// listener owner task that hands it accepted connections.
    fn spawn_connection_owner(
        services: ConnectionServices,
        owners: &OwnerShutdowns,
    ) -> (GatewayConnectionOwnerHandle, OwnerTask, GatewayListenerControl, OwnerTask) {
        let backlog = usize::try_from(services.limits.max_public_connections().get())
            .unwrap_or(CONNECTION_BACKLOG_CEILING)
            .min(CONNECTION_BACKLOG_CEILING);
        let (connection_sender, connection_messages) = mpsc::channel(backlog);
        let (drain, drain_watch) = watch::channel(None);
        let connections = GatewayConnectionOwnerHandle {
            stop_accepting: owners.listener.clone(),
            drain,
            force: CancellationToken::new(),
            drained: CancellationToken::new(),
            aborted: Arc::new(AtomicU32::new(0)),
        };
        let tokens = ConnectionTokens {
            shutdown: owners.connections.clone(),
            drain: drain_watch,
            force: connections.force.clone(),
            drained: connections.drained.clone(),
            aborted: Arc::clone(&connections.aborted),
        };
        let clock = Arc::clone(&services.clock);
        let task = tokio::spawn(run_connections(Arc::new(services), connection_messages, tokens));
        let (listener, listener_task) = GatewayListenerControl::spawn(
            owners.listener.clone(),
            AcceptPath { connections: connection_sender, clock },
        );
        (connections, task, listener, listener_task)
    }

    /// The manual-source, custody and Route Set owner tasks.
    fn spawn_supporting_owners(
        owners: SupportingOwners,
        custody_handle: &PublicCertifiedKeyCustodyHandle,
        clock: &Arc<dyn Clock>,
        shutdowns: &OwnerShutdowns,
    ) -> (OwnerTask, OwnerTask, OwnerTask) {
        let manual_source = tokio::spawn(manual_source_task(
            owners.manual,
            custody_handle.clone(),
            Arc::clone(clock),
            shutdowns.manual_source.clone(),
        ));
        let custody = tokio::spawn(task_result(owners.custody.run(shutdowns.custody.clone())));
        let route_set =
            tokio::spawn(task_result(owners.route_owner.run(shutdowns.route_set.clone())));
        (manual_source, custody, route_set)
    }
}

struct SupportingOwners {
    manual: ManualCertifiedKeySource,
    custody: PublicCertifiedKeyCustody,
    route_owner: PublicRouteSetOwner,
}

async fn task_result(
    task: impl std::future::Future<Output = ()>,
) -> Result<(), GatewayOwnerTaskError> {
    task.await;
    Ok(())
}

async fn manual_source_task(
    source: ManualCertifiedKeySource,
    custody: PublicCertifiedKeyCustodyHandle,
    clock: Arc<dyn Clock>,
    shutdown: CancellationToken,
) -> Result<(), GatewayOwnerTaskError> {
    source.run(custody, clock, shutdown).await;
    Ok(())
}

pub struct StartedGateway {
    handle: GatewayHandle,
    routes: GatewayRouteCommands,
    status: GatewayStatusReads,
}

impl StartedGateway {
    pub fn into_parts(self) -> (GatewayHandle, GatewayRouteCommands, GatewayStatusReads) {
        (self.handle, self.routes, self.status)
    }
}

/// Route command port (R-25): the write handle of the Public Route Set; it
/// exposes no status read.
#[derive(Clone)]
pub struct GatewayRouteCommands {
    routes: PublicRouteSetHandle,
}

impl GatewayRouteCommands {
    pub fn routes(&self) -> &PublicRouteSetHandle {
        &self.routes
    }
}

/// Read-only status port (R-25): redacted ObservationStore point reads; no
/// Route write capability is reachable from it.
#[derive(Clone)]
pub struct GatewayStatusReads {
    observations: Arc<dyn ObservationStore>,
    node_id: NodeId,
    certified_key_id: overdrive_core::public_ingress::PublicCertifiedKeyId,
}

impl GatewayStatusReads {
    pub async fn application_status(
        &self,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, GatewayStatusReadError> {
        self.observations
            .gateway_application_status_row(&self.node_id)
            .await
            .map_err(GatewayStatusReadError::Application)
    }
    pub async fn certified_key_status(
        &self,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, GatewayStatusReadError> {
        self.observations
            .public_certified_key_status_row(&self.certified_key_id)
            .await
            .map_err(GatewayStatusReadError::CertifiedKey)
    }
}

#[derive(Debug, Error)]
pub enum GatewayStatusReadError {
    #[error("Gateway Application status read failed: {0}")]
    Application(ObservationStoreError),
    #[error("public certified-key status read failed: {0}")]
    CertifiedKey(ObservationStoreError),
}

type OwnerTask = JoinHandle<Result<(), GatewayOwnerTaskError>>;

pub(crate) enum GatewayOwnerTaskError {
    Failed,
    ChannelClosed,
}

/// The owner tasks of one started gateway. The connections task joins this
/// tree with the accept loop.
pub(crate) struct GatewayTaskTree {
    listener: Option<OwnerTask>,
    connections: Option<OwnerTask>,
    application: OwnerTask,
    manual_source: OwnerTask,
    custody: OwnerTask,
    route_set: OwnerTask,
}

impl GatewayTaskTree {
    /// Joins one task exactly once against the remaining deadline; at expiry
    /// the task is aborted and awaited.
    async fn join_one(
        mut handle: OwnerTask,
        task: GatewayTaskName,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Option<GatewayTaskFailure> {
        let remaining = deadline.saturating_duration_since(clock.now());
        let joined = tokio::select! {
            joined = &mut handle => joined,
            () = clock.sleep(remaining) => {
                handle.abort();
                let _ = (&mut handle).await;
                return Some(GatewayTaskFailure { task, cause: GatewayTaskFailureCause::Timeout });
            }
        };
        let cause = match joined {
            Ok(Ok(())) => return None,
            Ok(Err(GatewayOwnerTaskError::Failed)) => GatewayTaskFailureCause::OwnerFailed,
            Ok(Err(GatewayOwnerTaskError::ChannelClosed)) => GatewayTaskFailureCause::ChannelClosed,
            Err(error) if error.is_panic() => GatewayTaskFailureCause::Panicked,
            Err(_) => GatewayTaskFailureCause::Cancelled,
        };
        Some(GatewayTaskFailure { task, cause })
    }

    /// Joins the listener task: once it returned no further accept can take a
    /// connection lease.
    pub(crate) async fn join_listener(
        &mut self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Option<GatewayTaskFailure> {
        let handle = self.listener.take()?;
        Self::join_one(handle, GatewayTaskName::Listener, deadline, clock).await
    }

    /// Joins the connections owner task.
    pub(crate) async fn join_connections(
        &mut self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Option<GatewayTaskFailure> {
        let handle = self.connections.take()?;
        Self::join_one(handle, GatewayTaskName::Connections, deadline, clock).await
    }

    pub(crate) async fn join_remaining(
        self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Vec<GatewayTaskFailure> {
        let mut failures = Vec::new();
        let ordered = [
            (Some(self.application), GatewayTaskName::Application),
            (Some(self.manual_source), GatewayTaskName::ManualCertifiedKeySource),
            (Some(self.custody), GatewayTaskName::PublicCertifiedKeyCustody),
            (Some(self.route_set), GatewayTaskName::PublicRouteSet),
        ];
        for (handle, task) in ordered {
            if let Some(handle) = handle {
                failures.extend(Self::join_one(handle, task, deadline, clock).await);
            }
        }
        failures
    }
}

pub(crate) struct GatewayShutdownCapabilities {
    application: GatewayApplicationHandle,
    connections: GatewayConnectionOwnerHandle,
    /// Cleanup sweep and live counts only (R-29): shutdown holds no handle
    /// that can prepare an upstream socket.
    cleanup: Arc<dyn GatewayConnectCleanup>,
    cleanup_ledger: Arc<GatewayCleanupLedger>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    clock: Arc<dyn Clock>,
    owners: OwnerShutdowns,
}

/// One cancellation token per owner task.
struct OwnerShutdowns {
    connections: CancellationToken,
    application: CancellationToken,
    manual_source: CancellationToken,
    custody: CancellationToken,
    route_set: CancellationToken,
    listener: CancellationToken,
}

impl OwnerShutdowns {
    fn new() -> Self {
        Self {
            connections: CancellationToken::new(),
            application: CancellationToken::new(),
            manual_source: CancellationToken::new(),
            custody: CancellationToken::new(),
            route_set: CancellationToken::new(),
            listener: CancellationToken::new(),
        }
    }

    /// The listener goes first: the application owner's exit closes the
    /// listener command channel, which the listener must not read as a
    /// failure.
    fn cancel_all(&self) {
        self.listener.cancel();
        self.connections.cancel();
        self.application.cancel();
        self.manual_source.cancel();
        self.custody.cancel();
        self.route_set.cancel();
    }
}

pub struct GatewayHandle {
    tasks: GatewayTaskTree,
    shutdown: GatewayShutdownCapabilities,
}

impl GatewayHandle {
    /// Stop accepts, close the application, join admitted users, sweep
    /// intents and receipts, retire the closed application, cancel the owners
    /// and only then clear the volatile gateway identity.
    pub async fn shutdown(self, grace: Duration) -> GatewayShutdownReport {
        let Self { mut tasks, shutdown: capabilities } = self;
        let clock = Arc::clone(&capabilities.clock);
        let started = clock.now();
        let deadline = started + grace;
        let mut report = GatewayShutdownReport {
            elapsed: Duration::ZERO,
            forced_connections: 0,
            task_failures: Vec::new(),
            cleanup_failures: Vec::new(),
            application_errors: Vec::new(),
            identity_empty: false,
            residual_connect: None,
        };

        capabilities.connections.stop_accepting();
        report.task_failures.extend(tasks.join_listener(deadline, clock.as_ref()).await);
        let closed = capabilities.application.close_for_shutdown(deadline).await;
        match capabilities.connections.drain_then_force_join(deadline, clock.as_ref()).await {
            Ok(aborted) => {
                report.forced_connections = report.forced_connections.saturating_add(aborted);
            }
            Err(failure) => report.task_failures.push(failure),
        }
        capabilities.owners.connections.cancel();
        report.task_failures.extend(tasks.join_connections(deadline, clock.as_ref()).await);
        match closed {
            Ok(closed) => {
                for retention in closed.retentions {
                    if let Err(timeout) = retention.wait_zero(deadline, clock.as_ref()).await {
                        report.forced_connections =
                            report.forced_connections.saturating_add(timeout.connections);
                    }
                }
            }
            Err(error) => report.application_errors.push(error),
        }

        report.cleanup_failures =
            capabilities.cleanup_ledger.retry_all(capabilities.cleanup.as_ref());
        report.residual_connect =
            Self::sweep(capabilities.cleanup.as_ref(), &capabilities.cleanup_ledger);

        if let Err(error) = capabilities.application.retire_closed(deadline).await {
            report.application_errors.push(error);
        }

        capabilities.owners.cancel_all();
        report.task_failures.extend(tasks.join_remaining(deadline, clock.as_ref()).await);

        capabilities.identity.disable_after_drain();
        report.identity_empty = capabilities.identity.current().is_none();
        report.elapsed = clock.now().saturating_duration_since(started);
        report
    }

    /// The final sweep: every Gateway intent is cleaned, then the residual
    /// live counts are read. `None` when the connect path is empty.
    fn sweep(
        cleanup: &dyn GatewayConnectCleanup,
        ledger: &GatewayCleanupLedger,
    ) -> Option<GatewayResidualConnectState> {
        let mut sweep_failures = Vec::new();
        if let Err(error) = cleanup.cleanup_all_gateway_intents() {
            sweep_failures.push(GatewayConnectSweepFailure::CleanupAll(error));
        }
        let live_intents = match cleanup.live_intent_count() {
            Ok(count) => Some(count),
            Err(error) => {
                sweep_failures.push(GatewayConnectSweepFailure::IntentCount(error));
                None
            }
        };
        // A failed snapshot read is the unreadable `None`, not a zero.
        let live_receipts = cleanup
            .live_receipts()
            .ok()
            .map(|receipts| u32::try_from(receipts.len()).unwrap_or(u32::MAX));
        let ledger_entries = ledger.len();
        let empty = sweep_failures.is_empty()
            && live_intents == Some(0)
            && live_receipts == Some(0)
            && ledger_entries == 0;
        (!empty).then_some(GatewayResidualConnectState {
            live_intents,
            live_receipts,
            sweep_failures,
            ledger_entries,
        })
    }
}

#[derive(Debug)]
pub struct GatewayShutdownReport {
    /// Time the stage consumed on the injected clock.
    pub elapsed: Duration,
    pub forced_connections: u32,
    pub task_failures: Vec<GatewayTaskFailure>,
    pub cleanup_failures: Vec<GatewayCleanupFailure>,
    pub application_errors: Vec<GatewayApplicationCommandError>,
    pub identity_empty: bool,
    pub residual_connect: Option<GatewayResidualConnectState>,
}

impl GatewayShutdownReport {
    pub fn is_clean(&self) -> bool {
        self.forced_connections == 0
            && self.task_failures.is_empty()
            && self.cleanup_failures.is_empty()
            && self.application_errors.is_empty()
            && self.identity_empty
            && self.residual_connect.is_none()
    }
}

#[derive(Debug)]
pub struct GatewayResidualConnectState {
    pub live_intents: Option<u32>,
    pub live_receipts: Option<u32>,
    pub sweep_failures: Vec<GatewayConnectSweepFailure>,
    pub ledger_entries: u32,
}

#[derive(Debug)]
pub enum GatewayConnectSweepFailure {
    CleanupAll(GatewayConnectDataplaneError),
    IntentCount(GatewayConnectDataplaneError),
    // No receipt-count variant: the residual receipt count is `live_receipts()?.len()` (R-33); a
    // failed `live_receipts()` read leaves `GatewayResidualConnectState::live_receipts` as `None`,
    // so the report is not clean.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayTaskName {
    Listener,
    Connections,
    Application,
    ManualCertifiedKeySource,
    PublicCertifiedKeyCustody,
    PublicRouteSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayTaskFailureCause {
    OwnerFailed,
    ChannelClosed,
    Timeout,
    Cancelled,
    Panicked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayTaskFailure {
    pub task: GatewayTaskName,
    pub cause: GatewayTaskFailureCause,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayCleanupFailure {
    ConnectIntent { count: u32 },
    Receipt { count: u32 },
    LedgerSaturated { attempts: u64, capacity: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayApplicationCommandPhase {
    Close,
    Retire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GatewayApplicationCommandError {
    #[error("Gateway Application owner unavailable")]
    OwnerUnavailable,
    #[error("Gateway Application command timed out during {phase:?}")]
    Timeout { phase: GatewayApplicationCommandPhase },
}

#[derive(Debug, Error)]
pub enum GatewayBootError {
    #[error("gateway configuration failed: {0}")]
    Configuration(GatewayConfigError),
    #[error("Route store failed: {0}")]
    RouteStore(PublicRouteSetError),
    #[error("public certified-key custody failed: {0}")]
    CertifiedKey(PublicCertifiedKeyError),
    #[error("manual public certified-key source failed: {0}")]
    ManualCertifiedKey(PublicCertifiedKeyError),
}

// ---------------------------------------------------------------------------
// Listener control
// ---------------------------------------------------------------------------

enum ListenerCommand {
    Bind {
        address: SocketAddrV4,
        admission: Arc<GatewayAdmission>,
        reply: oneshot::Sender<Result<(), std::io::Error>>,
    },
    Unbind {
        reply: oneshot::Sender<Result<(), std::io::Error>>,
    },
}

/// What the listener task needs to hand an accepted connection on.
struct AcceptPath {
    connections: mpsc::Sender<AcceptedConnection>,
    clock: Arc<dyn Clock>,
}

/// One admitted public connection on its way to the connections owner.
struct AcceptedConnection {
    socket: OwnedFd,
    peer: Ipv4Addr,
    lease: GatewayConnectionLease,
}

/// The bound socket and the admission it hands connections to.
struct BoundListener {
    socket: TcpListener,
    admission: Arc<GatewayAdmission>,
}

/// One listener command owner: the listener task owns the bound socket and its
/// accept loop.
pub(crate) struct GatewayListenerControl {
    commands: mpsc::Sender<ListenerCommand>,
    bound: Arc<AtomicBool>,
}

impl GatewayListenerControl {
    fn spawn(shutdown: CancellationToken, accept: AcceptPath) -> (Self, OwnerTask) {
        let (commands, receiver) = mpsc::channel(APPLICATION_COMMAND_CAPACITY);
        let bound = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(Self::run(receiver, Arc::clone(&bound), shutdown, accept));
        (Self { commands, bound }, task)
    }

    async fn run(
        mut receiver: mpsc::Receiver<ListenerCommand>,
        bound: Arc<AtomicBool>,
        shutdown: CancellationToken,
        accept: AcceptPath,
    ) -> Result<(), GatewayOwnerTaskError> {
        let mut held: Option<BoundListener> = None;
        loop {
            let command = tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    drop(held.take());
                    bound.store(false, Ordering::Release);
                    return Ok(());
                }
                (stream, peer) = Self::accept_next(held.as_ref(), accept.clock.as_ref()) => {
                    if let Some(listener) = held.as_ref() {
                        Self::admit(listener, stream, peer, &accept).await;
                    }
                    continue;
                }
                command = receiver.recv() => command,
            };
            match command {
                None => return Err(GatewayOwnerTaskError::ChannelClosed),
                Some(ListenerCommand::Bind { address, admission, reply }) => {
                    let bound_now = TcpListener::bind(address).await.map(|socket| {
                        drop(held.replace(BoundListener { socket, admission }));
                        bound.store(true, Ordering::Release);
                    });
                    let _ = reply.send(bound_now);
                }
                Some(ListenerCommand::Unbind { reply }) => {
                    drop(held.take());
                    bound.store(false, Ordering::Release);
                    let _ = reply.send(Ok(()));
                }
            }
        }
    }

    /// The next accepted socket; never ready while nothing is bound. An
    /// `accept(2)` failure (descriptor exhaustion, an aborted connection) is
    /// logged and followed by a bounded wait on the injected clock before the
    /// next attempt. The wait is inside this future, so the shutdown and
    /// command arms of the caller's `select!` still win during it.
    async fn accept_next(
        listener: Option<&BoundListener>,
        clock: &dyn Clock,
    ) -> (tokio::net::TcpStream, SocketAddr) {
        let Some(listener) = listener else {
            return std::future::pending().await;
        };
        loop {
            match listener.socket.accept().await {
                Ok(accepted) => return accepted,
                Err(error) => {
                    tracing::warn!(
                        name: "gateway.listener.accept_failed",
                        error = %error,
                        "accept failed; retrying after the backoff",
                    );
                    clock.sleep(ACCEPT_FAILURE_BACKOFF).await;
                }
            }
        }
    }

    /// Admit one accepted socket against the Current generation and pass it to
    /// the connections owner. A socket the admission refuses is closed.
    async fn admit(
        listener: &BoundListener,
        stream: tokio::net::TcpStream,
        peer: SocketAddr,
        accept: &AcceptPath,
    ) {
        let SocketAddr::V4(peer) = peer else {
            tracing::debug!(
                name: "gateway.listener.non_ipv4_peer_closed",
                peer = %peer,
                "an accepted connection from a non-IPv4 peer is closed",
            );
            return;
        };
        let now = UnixInstant::from_clock(accept.clock.as_ref());
        let lease = match listener.admission.acquire_connection(now) {
            Ok(lease) => lease,
            Err(refusal) => {
                tracing::debug!(
                    name: "gateway.listener.admission_refused",
                    peer = %peer,
                    refusal = ?refusal,
                    "admission refused the connection; it is closed",
                );
                return;
            }
        };
        let std_stream = match stream.into_std() {
            Ok(std_stream) => std_stream,
            Err(error) => {
                tracing::debug!(
                    name: "gateway.listener.into_std_failed",
                    peer = %peer,
                    error = %error,
                    "an accepted socket could not leave the reactor; it is closed",
                );
                return;
            }
        };
        let handed_over = accept
            .connections
            .send(AcceptedConnection { socket: OwnedFd::from(std_stream), peer: *peer.ip(), lease })
            .await;
        if handed_over.is_err() {
            tracing::debug!(
                name: "gateway.listener.connections_owner_gone",
                peer = %peer,
                "the connections owner is gone; the accepted connection is closed",
            );
        }
    }

    pub(crate) async fn bind(
        &self,
        address: SocketAddrV4,
        admission: Arc<GatewayAdmission>,
    ) -> Result<(), std::io::Error> {
        self.request(|reply| ListenerCommand::Bind { address, admission, reply }).await
    }

    pub(crate) async fn unbind(&self) -> Result<(), std::io::Error> {
        self.request(|reply| ListenerCommand::Unbind { reply }).await
    }

    pub(crate) fn is_bound(&self) -> bool {
        self.bound.load(Ordering::Acquire)
    }

    async fn request(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<(), std::io::Error>>) -> ListenerCommand,
    ) -> Result<(), std::io::Error> {
        let (reply, answer) = oneshot::channel();
        self.commands.send(command(reply)).await.map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        answer.await.map_err(|_| std::io::ErrorKind::BrokenPipe)?
    }
}

// ---------------------------------------------------------------------------
// Connections owner
// ---------------------------------------------------------------------------

/// Bound on the accept-to-owner hand-off queue.
const CONNECTION_BACKLOG_CEILING: usize = 1024;

/// The narrow handle shutdown holds on the connections owner.
pub(crate) struct GatewayConnectionOwnerHandle {
    /// Cancelled when accepting stops: the listener task ends.
    stop_accepting: CancellationToken,
    drain: watch::Sender<Option<Instant>>,
    force: CancellationToken,
    drained: CancellationToken,
    aborted: Arc<AtomicU32>,
}

impl GatewayConnectionOwnerHandle {
    /// No further connection is accepted: the listener task ends.
    pub(crate) fn stop_accepting(&self) {
        self.stop_accepting.cancel();
    }

    /// Begin the drain with `deadline` as its one deadline: idle connections
    /// close at once and in-flight exchanges run on. At `deadline` on `clock`
    /// every connection task still running is aborted and awaited, so its
    /// leases, legs and cleanup guards have run. Returns the number of
    /// connection tasks aborted.
    pub(crate) async fn drain_then_force_join(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<u32, GatewayTaskFailure> {
        self.drain.send_replace(Some(deadline));
        let remaining = deadline.saturating_duration_since(clock.now());
        tokio::select! {
            biased;
            () = self.drained.cancelled() => {}
            () = clock.sleep(remaining) => {
                self.force.cancel();
                self.drained.cancelled().await;
            }
        }
        Ok(self.aborted.load(Ordering::Acquire))
    }
}

/// Cancels `drained` however the owner task ends.
struct DrainedOnDrop(CancellationToken);

impl Drop for DrainedOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// What the connections owner task observes.
struct ConnectionTokens {
    shutdown: CancellationToken,
    drain: watch::Receiver<Option<Instant>>,
    force: CancellationToken,
    drained: CancellationToken,
    aborted: Arc<AtomicU32>,
}

/// The connections owner: spawns one task per admitted connection into its
/// `JoinSet`, reaps them, and once the drain began takes no new ones. It ends
/// when the drain emptied it, at the force-abort, or at shutdown.
async fn run_connections(
    services: Arc<ConnectionServices>,
    mut messages: mpsc::Receiver<AcceptedConnection>,
    mut tokens: ConnectionTokens,
) -> Result<(), GatewayOwnerTaskError> {
    let _drained = DrainedOnDrop(tokens.drained.clone());
    let mut connections: JoinSet<()> = JoinSet::new();
    let mut intake = true;
    loop {
        if !intake && connections.is_empty() {
            return Ok(());
        }
        tokio::select! {
            biased;
            () = tokens.force.cancelled() => {
                connections.abort_all();
                while let Some(joined) = connections.join_next().await {
                    if joined.is_err_and(|error| error.is_cancelled()) {
                        tokens.aborted.fetch_add(1, Ordering::AcqRel);
                    }
                }
                return Ok(());
            }
            () = tokens.shutdown.cancelled() => {
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                return Ok(());
            }
            changed = tokens.drain.changed(), if intake => {
                intake = false;
                if changed.is_ok() {
                    messages.close();
                    while messages.try_recv().is_ok() {}
                }
            }
            accepted = messages.recv(), if intake => match accepted {
                Some(AcceptedConnection { socket, peer, lease }) => {
                    connections.spawn(serve_connection(
                        Arc::clone(&services),
                        socket,
                        peer,
                        lease,
                        DrainSignal::new(tokens.drain.clone()),
                    ));
                }
                None => intake = false,
            },
            Some(joined) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = joined
                    && error.is_panic()
                {
                    tracing::error!(
                        name: "gateway.connection.task_panicked",
                        error = %error,
                        "a connection task panicked",
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Application commands and handle
// ---------------------------------------------------------------------------

pub(crate) enum GatewayApplicationCommand {
    CloseForShutdown {
        deadline: Instant,
        reply:
            oneshot::Sender<Result<GatewayApplicationCloseResult, GatewayApplicationCommandError>>,
    },
    RetireClosed {
        deadline: Instant,
        reply: oneshot::Sender<Result<(), GatewayApplicationCommandError>>,
    },
}

pub(crate) struct GatewayApplicationCloseResult {
    pub(crate) retentions: Vec<Arc<GatewayGenerationRetention>>,
}

pub(crate) struct GatewayApplicationHandle {
    commands: mpsc::Sender<GatewayApplicationCommand>,
    clock: Arc<dyn Clock>,
}

impl GatewayApplicationHandle {
    pub(crate) async fn close_for_shutdown(
        &self,
        deadline: Instant,
    ) -> Result<GatewayApplicationCloseResult, GatewayApplicationCommandError> {
        let (reply, answer) = oneshot::channel();
        self.exchange(
            GatewayApplicationCommand::CloseForShutdown { deadline, reply },
            answer,
            deadline,
            GatewayApplicationCommandPhase::Close,
        )
        .await?
    }

    pub(crate) async fn retire_closed(
        &self,
        deadline: Instant,
    ) -> Result<(), GatewayApplicationCommandError> {
        let (reply, answer) = oneshot::channel();
        self.exchange(
            GatewayApplicationCommand::RetireClosed { deadline, reply },
            answer,
            deadline,
            GatewayApplicationCommandPhase::Retire,
        )
        .await?
    }

    /// Send then await the reply, both raced against the absolute deadline on
    /// the injected clock. Closure of either channel is `OwnerUnavailable`.
    async fn exchange<T>(
        &self,
        command: GatewayApplicationCommand,
        answer: oneshot::Receiver<T>,
        deadline: Instant,
        phase: GatewayApplicationCommandPhase,
    ) -> Result<T, GatewayApplicationCommandError> {
        let remaining = deadline.saturating_duration_since(self.clock.now());
        let exchange = async {
            self.commands
                .send(command)
                .await
                .map_err(|_| GatewayApplicationCommandError::OwnerUnavailable)?;
            answer.await.map_err(|_| GatewayApplicationCommandError::OwnerUnavailable)
        };
        tokio::select! {
            result = exchange => result,
            () = self.clock.sleep(remaining) => {
                Err(GatewayApplicationCommandError::Timeout { phase })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The Gateway Application owner
// ---------------------------------------------------------------------------

/// Why a gate attempt ended without promoting.
#[derive(Debug)]
enum GateFailure {
    Identity(GatewayIdentityLifecycleError),
    ConnectPath(GatewayUpstreamConnectError),
    ConnectProbe(GatewayConnectDataplaneError),
    Timeout,
}

struct GateReport {
    token: u64,
    result: Result<(), GateFailure>,
}

const PHASE_IDENTITY: u8 = 0;
const PHASE_CONNECT: u8 = 1;

/// The staged candidate with its gate attempt.
struct StagedGate {
    candidate: GatewayApplicationCandidate,
    token: u64,
    phase: Arc<AtomicU8>,
    /// Cooperative cancel for the gate attempt and its deadline timer.
    cancel: CancellationToken,
}

/// The newest status the owner wrote, and the issue-failure base read from
/// the prior row (R-52).
#[derive(Default)]
struct StatusWriter {
    last_written: Option<GatewayApplicationStatusRowV1>,
    stamp: Option<LogicalTimestamp>,
    base: Option<(u64, Option<overdrive_core::ca::issue_failure::SvidIssueFailure>)>,
    dirty: bool,
}

pub(crate) struct GatewayApplicationOwner {
    node_id: NodeId,
    intent: Arc<dyn IntentStore>,
    certified_key: PublicCertifiedKeyCustodyHandle,
    frontend: Arc<dyn ServiceFrontendResolve>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    connector: Arc<GatewayUpstreamConnector>,
    /// Driven once per boot by the first gate that reaches it.
    dataplane: Arc<dyn GatewayConnectDataplane>,
    connect_proven: Arc<AtomicBool>,
    admission: Arc<GatewayAdmission>,
    listener: GatewayListenerControl,
    observations: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
    limits: Arc<GatewayLimits>,
    bind: SocketAddrV4,
    commands: mpsc::Receiver<GatewayApplicationCommand>,
    shutdown: CancellationToken,
    owner_wake: Arc<Notify>,
    staged: Option<StagedGate>,
    current: Option<Arc<GatewayApplicationSnapshot>>,
    draining: BTreeMap<GatewayApplicationGenerationId, Arc<GatewayApplicationSnapshot>>,
    /// The generation whose gate failed; retried only when an input changes.
    failed: Option<GatewayApplicationGenerationId>,
    unavailable: Option<GatewayApplicationUnavailableCause>,
    connect_ready: bool,
    next_token: u64,
    gates: JoinSet<()>,
    status: StatusWriter,
    /// Whether a Route has ever been observed by this process. Until one is,
    /// and no prior row exists, the owner writes no status row (R-58).
    route_seen: bool,
}

/// What the application owner is wired to; everything else starts empty.
struct OwnerWiring {
    node_id: NodeId,
    intent: Arc<dyn IntentStore>,
    certified_key: PublicCertifiedKeyCustodyHandle,
    frontend: Arc<dyn ServiceFrontendResolve>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    connector: Arc<GatewayUpstreamConnector>,
    dataplane: Arc<dyn GatewayConnectDataplane>,
    admission: Arc<GatewayAdmission>,
    listener: GatewayListenerControl,
    observations: Arc<dyn ObservationStore>,
    clock: Arc<dyn Clock>,
    limits: Arc<GatewayLimits>,
    bind: SocketAddrV4,
    commands: mpsc::Receiver<GatewayApplicationCommand>,
    shutdown: CancellationToken,
}

impl GatewayApplicationOwner {
    /// An owner at boot: no generation staged or current, no Route seen.
    fn fresh(wiring: OwnerWiring) -> Self {
        Self {
            node_id: wiring.node_id,
            intent: wiring.intent,
            certified_key: wiring.certified_key,
            frontend: wiring.frontend,
            identity: wiring.identity,
            connector: wiring.connector,
            dataplane: wiring.dataplane,
            connect_proven: Arc::new(AtomicBool::new(false)),
            admission: wiring.admission,
            listener: wiring.listener,
            observations: wiring.observations,
            clock: wiring.clock,
            limits: wiring.limits,
            bind: wiring.bind,
            commands: wiring.commands,
            shutdown: wiring.shutdown,
            owner_wake: Arc::new(Notify::new()),
            staged: None,
            current: None,
            draining: BTreeMap::new(),
            failed: None,
            unavailable: None,
            connect_ready: false,
            next_token: 0,
            gates: JoinSet::new(),
            route_seen: false,
            status: StatusWriter::default(),
        }
    }

    /// Subscribe-before-list, then serve inputs until shutdown.
    pub(crate) async fn run(mut self) -> Result<(), GatewayOwnerTaskError> {
        let mut route_events = self
            .intent
            .watch(ROUTE_SET_KEY.as_bytes())
            .await
            .map_err(|_| GatewayOwnerTaskError::Failed)?;
        let mut key_events = self.certified_key.subscribe();
        let mut identity_events = self.identity.subscribe();
        let mut last_identity = identity_events.borrow_and_update().clone();
        let (report_sender, mut reports) = mpsc::channel::<GateReport>(GATE_REPORT_CAPACITY);
        let shutdown = self.shutdown.clone();
        let wake = Arc::clone(&self.owner_wake);
        let clock = Arc::clone(&self.clock);

        // Shutdown cancels this owner's token before any dependency's
        // (custody included), so a closed input observed after cancellation is
        // the requested shutdown, not a failure. Without cancellation it is a
        // genuine unrequested dependency loss.
        let input_closed = || {
            if shutdown.is_cancelled() { Ok(()) } else { Err(GatewayOwnerTaskError::ChannelClosed) }
        };

        self.evaluate(&report_sender).await;
        loop {
            let retry = self.status.dirty;
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(()),
                command = self.commands.recv() => match command {
                    None => return input_closed(),
                    Some(command) => self.command(command).await,
                },
                report = reports.recv() => {
                    if let Some(report) = report {
                        self.gate_report(report, &report_sender).await;
                    }
                }
                event = std::future::poll_fn(|cx| std::pin::Pin::new(&mut *route_events).poll_next(cx)) => match event {
                    None => return input_closed(),
                    Some(IntentSubscriptionEvent::Changed { .. }) => {
                        self.failed = None;
                        self.evaluate(&report_sender).await;
                    }
                    Some(IntentSubscriptionEvent::Lagged { .. }) => {
                        self.close_application(GatewayApplicationUnavailableCause::WatchGap).await;
                        self.failed = None;
                        self.evaluate(&report_sender).await;
                    }
                },
                changed = key_events.changed() => {
                    if changed.is_err() {
                        return input_closed();
                    }
                    key_events.borrow_and_update();
                    self.failed = None;
                    self.evaluate(&report_sender).await;
                }
                changed = identity_events.changed() => {
                    if changed.is_err() {
                        return input_closed();
                    }
                    let held = identity_events.borrow_and_update().clone();
                    if held != last_identity {
                        self.failed = None;
                        last_identity = held;
                        self.evaluate(&report_sender).await;
                    }
                    self.publish().await;
                }
                () = wake.notified() => {
                    self.retire_drained();
                    self.publish().await;
                }
                () = async {
                    if retry {
                        clock.sleep(STATUS_RETRY).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => self.publish().await,
            }
        }
    }

    async fn command(&mut self, command: GatewayApplicationCommand) {
        match command {
            GatewayApplicationCommand::CloseForShutdown { deadline, reply } => {
                let result = if self.clock.now() > deadline {
                    Err(GatewayApplicationCommandError::Timeout {
                        phase: GatewayApplicationCommandPhase::Close,
                    })
                } else {
                    Ok(self.close_for_shutdown().await)
                };
                let _ = reply.send(result);
            }
            GatewayApplicationCommand::RetireClosed { deadline, reply } => {
                let result = if self.clock.now() > deadline {
                    Err(GatewayApplicationCommandError::Timeout {
                        phase: GatewayApplicationCommandPhase::Retire,
                    })
                } else {
                    self.draining.clear();
                    self.publish().await;
                    Ok(())
                };
                let _ = reply.send(result);
            }
        }
    }

    /// Discard `staged`, drop the in-flight gates, unbind, close every
    /// retention and clear admission. Returns all retentions.
    async fn close_for_shutdown(&mut self) -> GatewayApplicationCloseResult {
        self.drop_staged();
        self.unbind().await;
        self.admission.current.store(None);
        if let Some(current) = self.current.take() {
            self.draining.insert(current.applied().application_generation(), current);
        }
        let retentions = self
            .draining
            .values()
            .map(|snapshot| {
                snapshot.retention.close();
                Arc::clone(&snapshot.retention)
            })
            .collect();
        self.publish().await;
        GatewayApplicationCloseResult { retentions }
    }

    fn drop_staged(&mut self) {
        if let Some(staged) = self.staged.take() {
            staged.cancel.cancel();
        }
    }

    async fn unbind(&mut self) {
        if self.listener.is_bound() {
            let _ = self.listener.unbind().await;
        }
    }

    /// Stop accepts and unbind before clearing Current, which drains.
    async fn close_application(&mut self, cause: GatewayApplicationUnavailableCause) {
        self.drop_staged();
        self.unbind().await;
        if let Some(current) = self.current.take() {
            current.retention.close();
            self.admission.current.store(None);
            self.draining.insert(current.applied().application_generation(), current);
        }
        self.retire_drained();
        self.unavailable = Some(cause);
        self.publish().await;
    }

    /// Whether a status row from an earlier process is still stored, so a
    /// stale row is replaced instead of left standing.
    async fn prior_row_exists(&self) -> bool {
        // An unreadable store is reported by the next projection's retry, so
        // the safe answer here is that a row may exist.
        self.observations
            .gateway_application_status_row(&self.node_id)
            .await
            .map_or(true, |row| row.is_some())
    }

    /// Retire every draining generation whose retained connections and
    /// requests have all ended.
    fn retire_drained(&mut self) {
        self.draining.retain(|_, snapshot| !snapshot.retention.is_drained());
    }

    /// Resolve the three inputs and stage a candidate when they changed.
    async fn evaluate(&mut self, reports: &mpsc::Sender<GateReport>) {
        let route = match PublicRouteSetOwner::read(self.intent.as_ref()).await {
            Ok(Some(route)) => {
                self.route_seen = true;
                Arc::new(route)
            }
            Ok(None) => {
                if self.status.last_written.is_none() && !self.prior_row_exists().await {
                    // Nothing was ever projected and no Route exists: there
                    // is no Application fact to report yet.
                    return;
                }
                return self
                    .close_application(GatewayApplicationUnavailableCause::RouteAbsent)
                    .await;
            }
            Err(_) => {
                return self
                    .close_application(GatewayApplicationUnavailableCause::RouteUnreadable)
                    .await;
            }
        };
        let certified_key = match self.certified_key.snapshot() {
            PublicCertifiedKeyAvailability::Usable(snapshot) => Arc::new(snapshot),
            PublicCertifiedKeyAvailability::Absent => {
                return self
                    .close_application(GatewayApplicationUnavailableCause::CertifiedKeyAbsent)
                    .await;
            }
            PublicCertifiedKeyAvailability::Unusable { .. } => {
                return self
                    .close_application(GatewayApplicationUnavailableCause::CertifiedKeyUnusable)
                    .await;
            }
        };
        let Ok(frontend) = self.frontend.resolve(route.service_listener()).await else {
            return self
                .close_application(GatewayApplicationUnavailableCause::FrontendUnresolved)
                .await;
        };
        let candidate = match GatewayApplicationCandidate::new(route, frontend, certified_key) {
            Ok(candidate) => candidate,
            Err(GatewayApplicationError::CertifiedKeyHostnameMismatch) => {
                return self
                    .close_application(GatewayApplicationUnavailableCause::CertifiedKeyUnusable)
                    .await;
            }
            Err(_) => {
                return self
                    .close_application(GatewayApplicationUnavailableCause::CertifiedKeyAbsent)
                    .await;
            }
        };
        let generation = candidate.generation;
        let current = self.current.as_ref().map(|c| c.applied().application_generation());
        if current == Some(generation) {
            self.drop_staged();
            self.unavailable = None;
            return self.publish().await;
        }
        let staged = self.staged.as_ref().map(|staged| staged.candidate.generation);
        if staged == Some(generation) || self.failed == Some(generation) {
            return;
        }
        self.stage(candidate, reports).await;
    }

    /// Publish the exact frontend as Staged and start one gate attempt against
    /// one absolute deadline.
    async fn stage(
        &mut self,
        candidate: GatewayApplicationCandidate,
        reports: &mpsc::Sender<GateReport>,
    ) {
        self.drop_staged();
        self.next_token += 1;
        let token = self.next_token;
        let deadline = self.clock.now() + self.limits.application_gate_deadline();
        let phase = Arc::new(AtomicU8::new(PHASE_IDENTITY));
        let cancel = CancellationToken::new();
        let gate = Gate {
            connector: Arc::clone(&self.connector),
            dataplane: Arc::clone(&self.dataplane),
            connect_proven: Arc::clone(&self.connect_proven),
            identity: Arc::clone(&self.identity),
            clock: Arc::clone(&self.clock),
            frontend: candidate.frontend,
            deadline,
            phase: Arc::clone(&phase),
            token,
            reports: reports.clone(),
            cancel: cancel.clone(),
        };
        self.gates.spawn(gate.run());
        tokio::spawn(Self::gate_timer(
            Arc::clone(&self.clock),
            deadline,
            token,
            reports.clone(),
            cancel.clone(),
        ));
        self.unavailable = None;
        self.staged = Some(StagedGate { candidate, token, phase, cancel });
        self.publish().await;
    }

    async fn gate_timer(
        clock: Arc<dyn Clock>,
        deadline: Instant,
        token: u64,
        reports: mpsc::Sender<GateReport>,
        cancel: CancellationToken,
    ) {
        tokio::select! {
            biased;
            () = cancel.cancelled() => {}
            () = clock.sleep(deadline.saturating_duration_since(clock.now())) => {
                let _ = reports.send(GateReport { token, result: Err(GateFailure::Timeout) }).await;
            }
        }
    }

    /// A report for any token but the staged one is a late result of a
    /// superseded or already-completed attempt: ignored, never delivered.
    async fn gate_report(&mut self, report: GateReport, reports: &mpsc::Sender<GateReport>) {
        if self.staged.as_ref().map(|staged| staged.token) != Some(report.token) {
            return;
        }
        let Some(staged) = self.staged.take() else { return };
        staged.cancel.cancel();
        match report.result {
            Ok(()) => self.promote(staged.candidate, reports).await,
            Err(failure) => {
                let cause = match failure {
                    GateFailure::Identity(error) => {
                        tracing::warn!(
                            name: "gateway.application.gate_identity_failed",
                            error = %error,
                            "gateway application gate failed on gateway identity",
                        );
                        GatewayApplicationUnavailableCause::GatewayIdentityUnusable
                    }
                    GateFailure::ConnectPath(error) => {
                        tracing::warn!(
                            name: "gateway.application.gate_connect_path_failed",
                            error = %error,
                            "gateway application gate failed on the upstream connect path",
                        );
                        GatewayApplicationUnavailableCause::ConnectPathUnavailable
                    }
                    GateFailure::ConnectProbe(error) => {
                        tracing::warn!(
                            name: "gateway.application.gate_connect_probe_failed",
                            error = %error,
                            "gateway application gate failed on the connect dataplane probe",
                        );
                        GatewayApplicationUnavailableCause::ConnectPathUnavailable
                    }
                    GateFailure::Timeout => {
                        if staged.phase.load(Ordering::Acquire) == PHASE_IDENTITY {
                            GatewayApplicationUnavailableCause::GatewayIdentityUnusable
                        } else {
                            GatewayApplicationUnavailableCause::ConnectPathUnavailable
                        }
                    }
                };
                self.reject(staged.candidate.generation, cause).await;
            }
        }
    }

    /// A failed gate removes only the staged candidate.
    async fn reject(
        &mut self,
        generation: GatewayApplicationGenerationId,
        cause: GatewayApplicationUnavailableCause,
    ) {
        self.failed = Some(generation);
        if cause == GatewayApplicationUnavailableCause::ConnectPathUnavailable {
            self.connect_ready = false;
        }
        if self.current.is_none() {
            self.unavailable = Some(cause);
        }
        self.publish().await;
    }

    /// Promote after the gate: recheck identity and key, swap Current, move
    /// the prior Current to Draining, then bind if needed.
    async fn promote(
        &mut self,
        candidate: GatewayApplicationCandidate,
        reports: &mpsc::Sender<GateReport>,
    ) {
        let generation = candidate.generation;
        let identity_usable = self
            .identity
            .current()
            .is_some_and(|facts| facts.not_after > UnixInstant::from_clock(self.clock.as_ref()));
        if !identity_usable {
            return self
                .reject(generation, GatewayApplicationUnavailableCause::GatewayIdentityUnusable)
                .await;
        }
        let key_current = matches!(
            self.certified_key.snapshot(),
            PublicCertifiedKeyAvailability::Usable(ref snapshot)
                if snapshot.generation() == candidate.certified_key.generation()
        );
        if !key_current {
            return self.evaluate(reports).await;
        }
        let snapshot = Arc::new(GatewayApplicationSnapshot {
            applied: GatewayAppliedGeneration::from_promotion(generation),
            route: candidate.route,
            frontend: candidate.frontend,
            certified_key: candidate.certified_key,
            retention: GatewayGenerationRetention::new(generation, Arc::clone(&self.owner_wake)),
            budget: Arc::clone(self.admission.budget()),
        });
        let prior = self.current.take();
        if let Some(prior) = &prior {
            prior.retention.close();
        }
        self.admission.current.store(Some(Arc::clone(&snapshot)));
        self.current = Some(snapshot);
        if let Some(prior) = prior {
            self.draining.insert(prior.applied().application_generation(), prior);
        }
        self.connect_ready = true;
        self.unavailable = None;
        self.failed = None;
        if !self.listener.is_bound()
            && let Err(error) = self.listener.bind(self.bind, Arc::clone(&self.admission)).await
        {
            tracing::warn!(
                name: "gateway.listener.bind_failed",
                kind = ?error.kind(),
                "public listener bind failed; the promoted generation is removed",
            );
            self.close_application(GatewayApplicationUnavailableCause::ListenerBindFailed).await;
            self.failed = Some(generation);
            return;
        }
        self.retire_drained();
        self.publish().await;
    }

    // -- status projection --------------------------------------------------

    fn classify_identity(&self) -> GatewayIdentityStatus {
        let Some(facts) = self.identity.current() else {
            return GatewayIdentityStatus::Absent;
        };
        if facts.not_after <= UnixInstant::from_clock(self.clock.as_ref()) {
            return GatewayIdentityStatus::Unusable;
        }
        GatewayIdentityStatus::Current {
            epoch: facts.epoch,
            spiffe_id: facts.spiffe_id,
            serial: facts.serial,
            not_after: facts.not_after,
        }
    }

    fn generation_status(
        generation: GatewayApplicationGenerationId,
        route: &Route,
        frontend: ServiceFrontend,
    ) -> GatewayApplicationGenerationStatus {
        GatewayApplicationGenerationStatus {
            application_generation: generation,
            route_id: route.route_id().clone(),
            route_generation: route.route_generation(),
            service_key: ServiceKey::from_frontend(frontend),
        }
    }

    fn snapshot_status(
        snapshot: &GatewayApplicationSnapshot,
    ) -> GatewayApplicationGenerationStatus {
        Self::generation_status(
            snapshot.applied().application_generation(),
            snapshot.route(),
            snapshot.frontend(),
        )
    }

    /// The newest row value: application facts plus identity and the
    /// issue-failure occurrence (base from the prior row plus this process's
    /// record), with the previous stamp as a placeholder.
    fn project(
        &self,
        base: &(u64, Option<overdrive_core::ca::issue_failure::SvidIssueFailure>),
        updated_at: LogicalTimestamp,
    ) -> GatewayApplicationStatusRowV1 {
        let record = self.identity.issue_record();
        let (issue_failures, last_issue_failure) = record.map_or((base.0, base.1), |record| {
            (base.0.saturating_add(record.count.get()), Some(record.last))
        });
        GatewayApplicationStatusRowV1 {
            node_id: self.node_id.clone(),
            listener: if self.listener.is_bound() {
                GatewayListenerStatus::Bound { address: self.bind }
            } else {
                GatewayListenerStatus::Unbound
            },
            staged: self.staged.as_ref().map(|staged| {
                Self::generation_status(
                    staged.candidate.generation,
                    &staged.candidate.route,
                    staged.candidate.frontend,
                )
            }),
            current: self.current.as_deref().map(Self::snapshot_status),
            draining: self
                .draining
                .values()
                .map(|snapshot| Self::snapshot_status(snapshot))
                .collect(),
            unavailable: self.unavailable,
            gateway_identity: self.classify_identity(),
            issue_failures,
            last_issue_failure,
            connect_path: if self.connect_ready {
                GatewayConnectPathStatus::Ready
            } else {
                GatewayConnectPathStatus::Unavailable
            },
            updated_at,
        }
    }

    /// Write the newest projection when it differs from the last written one.
    /// A failed read or write keeps the owner dirty; the next wake or retry
    /// carries everything the owner holds.
    async fn publish(&mut self) {
        if self.status.base.is_none() {
            let Ok(prior) = self.observations.gateway_application_status_row(&self.node_id).await
            else {
                self.status.dirty = true;
                return;
            };
            if prior.is_none() && !self.route_seen {
                // R-58: no Route has been configured and no row exists from an
                // earlier boot, so the gateway's status is the absence of a
                // row. The identity and failure facts stay held for the first
                // row written after a Route is observed.
                return;
            }
            self.status.base = Some(
                prior
                    .as_ref()
                    .map_or((0, None), |row| (row.issue_failures, row.last_issue_failure)),
            );
            self.status.stamp = prior.map(|row| row.updated_at);
        }
        let Some(base) = self.status.base else { return };
        let stamp =
            LogicalTimestamp::dominating(0, self.node_id.clone(), self.status.stamp.as_ref());
        let row = self.project(&base, stamp.clone());
        if let Some(last) = &self.status.last_written {
            let mut comparable = row.clone();
            comparable.updated_at = last.updated_at.clone();
            if comparable == *last {
                self.status.dirty = false;
                return;
            }
        }
        match self.observations.write(ObservationWrite::GatewayApplicationStatus(row.clone())).await
        {
            Ok(()) => {
                self.status.stamp = Some(stamp);
                self.status.last_written = Some(row);
                self.status.dirty = false;
            }
            Err(_) => self.status.dirty = true,
        }
    }
}

/// One gate attempt: identity current, then the actual-frontend probe, reported by token. It runs detached from the owner
/// loop so a superseded attempt completes (and cleans up) on its own.
struct Gate {
    connector: Arc<GatewayUpstreamConnector>,
    dataplane: Arc<dyn GatewayConnectDataplane>,
    connect_proven: Arc<AtomicBool>,
    identity: Arc<dyn GatewayIdentityLifecycleControl>,
    clock: Arc<dyn Clock>,
    frontend: ServiceFrontend,
    deadline: Instant,
    phase: Arc<AtomicU8>,
    token: u64,
    reports: mpsc::Sender<GateReport>,
    cancel: CancellationToken,
}

impl Gate {
    /// Runs the attempt. A superseded attempt stops cooperatively before its
    /// next step; a probe already in flight runs to its own deadline so its
    /// intent and receipt are cleaned up, and its late result is never
    /// reported.
    async fn run(self) {
        let result = self.steps().await;
        if self.cancel.is_cancelled() {
            return;
        }
        let _ = self.reports.send(GateReport { token: self.token, result }).await;
    }

    /// The dataplane's own probe (residue sweep, sentinel connect) runs once
    /// per boot, before the first candidate's identity step.
    async fn prove_connect_dataplane(&self) -> Result<(), GateFailure> {
        if self.connect_proven.load(Ordering::Acquire) {
            return Ok(());
        }
        self.phase.store(PHASE_CONNECT, Ordering::Release);
        self.dataplane
            .probe(self.deadline, self.clock.as_ref())
            .await
            .map_err(GateFailure::ConnectProbe)?;
        self.connect_proven.store(true, Ordering::Release);
        self.phase.store(PHASE_IDENTITY, Ordering::Release);
        Ok(())
    }

    async fn steps(&self) -> Result<(), GateFailure> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        self.prove_connect_dataplane().await?;
        let identity = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Ok(()),
            identity = self.identity.ensure_current(self.deadline) => identity,
        };
        identity.map_err(GateFailure::Identity)?;
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        self.phase.store(PHASE_CONNECT, Ordering::Release);
        self.connector
            .probe(self.frontend, self.deadline)
            .await
            .map(|_| ())
            .map_err(GateFailure::ConnectPath)
    }
}

// ---------------------------------------------------------------------------
// Crate-private candidate, snapshot and retention
// ---------------------------------------------------------------------------

pub(crate) struct GatewayApplicationCandidate {
    generation: GatewayApplicationGenerationId,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
}

impl GatewayApplicationCandidate {
    pub(crate) fn new(
        route: Arc<Route>,
        frontend: ServiceFrontend,
        certified_key: Arc<PublicCertifiedKeySnapshot>,
    ) -> Result<Self, GatewayApplicationError> {
        if route.public_certified_key_id() != certified_key.certified_key_id() {
            return Err(GatewayApplicationError::CertifiedKeyReferenceMismatch);
        }
        if route.public_hostname() != certified_key.public_hostname() {
            return Err(GatewayApplicationError::CertifiedKeyHostnameMismatch);
        }
        let generation = GatewayApplicationGenerationId::from_inputs(
            route.route_generation(),
            frontend,
            certified_key.generation(),
        );
        Ok(Self { generation, route, frontend, certified_key })
    }
}

pub(crate) struct GatewayApplicationSnapshot {
    applied: GatewayAppliedGeneration,
    route: Arc<Route>,
    frontend: ServiceFrontend,
    certified_key: Arc<PublicCertifiedKeySnapshot>,
    retention: Arc<GatewayGenerationRetention>,
    budget: Arc<GatewayAdmissionBudget>,
}

impl GatewayApplicationSnapshot {
    pub(crate) const fn applied(&self) -> GatewayAppliedGeneration {
        self.applied
    }
    pub(crate) fn route(&self) -> &Route {
        &self.route
    }
    pub(crate) const fn frontend(&self) -> ServiceFrontend {
        self.frontend
    }
    pub(crate) fn certified_key(&self) -> &PublicCertifiedKeySnapshot {
        &self.certified_key
    }
}

pub(crate) struct GatewayGenerationRetention {
    generation: GatewayApplicationGenerationId,
    closed: AtomicBool,
    connections: AtomicU32,
    requests: AtomicU32,
    owner_wake: Arc<Notify>,
}

impl GatewayGenerationRetention {
    pub(crate) fn new(
        generation: GatewayApplicationGenerationId,
        owner_wake: Arc<Notify>,
    ) -> Arc<Self> {
        Arc::new(Self {
            generation,
            closed: AtomicBool::new(false),
            connections: AtomicU32::new(0),
            requests: AtomicU32::new(0),
            owner_wake,
        })
    }
    pub(crate) fn try_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationConnectionPermit, GatewayAdmissionError> {
        self.take_count(&self.connections)?;
        Ok(GatewayGenerationConnectionPermit { retention: Arc::clone(self) })
    }
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGenerationRequestPermit, GatewayAdmissionError> {
        self.take_count(&self.requests)?;
        Ok(GatewayGenerationRequestPermit { retention: Arc::clone(self) })
    }

    /// Count one more retained user, refused once the generation is closed.
    /// `closed` is tested after the increment as well, so a close that raced
    /// the increment rolls it back.
    fn take_count(&self, count: &AtomicU32) -> Result<(), GatewayAdmissionError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(GatewayAdmissionError::Unavailable);
        }
        count.fetch_add(1, Ordering::AcqRel);
        if self.closed.load(Ordering::Acquire) {
            count.fetch_sub(1, Ordering::AcqRel);
            self.owner_wake.notify_one();
            return Err(GatewayAdmissionError::Unavailable);
        }
        Ok(())
    }

    fn release(&self, count: &AtomicU32) {
        count.fetch_sub(1, Ordering::AcqRel);
        self.owner_wake.notify_one();
    }
    /// Refuse every later connection and request of this generation.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    /// Wait until no connection or request is retained, or the absolute
    /// `deadline` on `clock` passes (the exact remaining counts).
    pub(crate) async fn wait_zero(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayDrainTimeout> {
        loop {
            let released = self.owner_wake.notified();
            let (connections, requests) = self.retained();
            if connections == 0 && requests == 0 {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(clock.now());
            tokio::select! {
                () = released => {}
                () = clock.sleep(remaining) => {
                    let (connections, requests) = self.retained();
                    return Err(GatewayDrainTimeout {
                        generation: self.generation,
                        connections,
                        requests,
                    });
                }
            }
        }
    }
    pub(crate) fn counts(&self) -> (u32, u32) {
        todo!("SCAFFOLD: GatewayGenerationRetention::counts")
    }

    fn retained(&self) -> (u32, u32) {
        (self.connections.load(Ordering::Acquire), self.requests.load(Ordering::Acquire))
    }

    fn is_drained(&self) -> bool {
        self.retained() == (0, 0)
    }
}

pub(crate) struct GatewayGenerationConnectionPermit {
    retention: Arc<GatewayGenerationRetention>,
}

impl Drop for GatewayGenerationConnectionPermit {
    fn drop(&mut self) {
        self.retention.release(&self.retention.connections);
    }
}

pub(crate) struct GatewayGenerationRequestPermit {
    retention: Arc<GatewayGenerationRetention>,
}

impl Drop for GatewayGenerationRequestPermit {
    fn drop(&mut self) {
        self.retention.release(&self.retention.requests);
    }
}

// ---------------------------------------------------------------------------
// Admission budget, admission and leases
// ---------------------------------------------------------------------------

pub(crate) struct GatewayAdmissionBudget {
    limits: Arc<GatewayLimits>,
    public_connections: AtomicU32,
    inflight_connects: AtomicU32,
    inflight_requests: AtomicU32,
}

impl GatewayAdmissionBudget {
    pub(crate) fn new(limits: Arc<GatewayLimits>) -> Arc<Self> {
        Arc::new(Self {
            limits,
            public_connections: AtomicU32::new(0),
            inflight_connects: AtomicU32::new(0),
            inflight_requests: AtomicU32::new(0),
        })
    }
    pub(crate) fn try_public_connection(
        self: &Arc<Self>,
    ) -> Result<GatewayPublicConnectionPermit, GatewayAdmissionError> {
        let ceiling = self.limits.max_public_connections().get();
        let taken =
            self.public_connections.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < ceiling).then_some(held + 1)
            });
        match taken {
            Ok(_) => Ok(GatewayPublicConnectionPermit { budget: Arc::clone(self) }),
            Err(_) => todo!(
                "SCAFFOLD: GatewayAdmissionBudget::try_public_connection refusal arm (connection limit)"
            ),
        }
    }
    pub(crate) fn try_connect(
        self: &Arc<Self>,
    ) -> Result<GatewayConnectPermit, GatewayAdmissionError> {
        let ceiling = self.limits.max_inflight_connects().get();
        let taken =
            self.inflight_connects.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < ceiling).then_some(held + 1)
            });
        match taken {
            Ok(_) => Ok(GatewayConnectPermit { budget: Arc::clone(self) }),
            Err(_) => todo!(
                "SCAFFOLD: GatewayAdmissionBudget::try_connect refusal arm (admission refusal)"
            ),
        }
    }
    pub(crate) fn try_request(
        self: &Arc<Self>,
    ) -> Result<GatewayGlobalRequestPermit, GatewayAdmissionError> {
        let ceiling = self.limits.max_inflight_requests().get();
        let taken =
            self.inflight_requests.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < ceiling).then_some(held + 1)
            });
        match taken {
            Ok(_) => Ok(GatewayGlobalRequestPermit { budget: Arc::clone(self) }),
            Err(_) => {
                todo!("SCAFFOLD: GatewayAdmissionBudget::try_request refusal arm (request limit)")
            }
        }
    }
}

pub(crate) struct GatewayPublicConnectionPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

impl Drop for GatewayPublicConnectionPermit {
    fn drop(&mut self) {
        self.budget.public_connections.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct GatewayConnectPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

impl Drop for GatewayConnectPermit {
    fn drop(&mut self) {
        self.budget.inflight_connects.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct GatewayGlobalRequestPermit {
    budget: Arc<GatewayAdmissionBudget>,
}

impl Drop for GatewayGlobalRequestPermit {
    fn drop(&mut self) {
        self.budget.inflight_requests.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct GatewayAdmission {
    current: Arc<ArcSwapOption<GatewayApplicationSnapshot>>,
    budget: Arc<GatewayAdmissionBudget>,
}

impl GatewayAdmission {
    /// `current` empty. The one constructor (DG-10).
    pub(crate) fn new(budget: Arc<GatewayAdmissionBudget>) -> Arc<Self> {
        Arc::new(Self { current: Arc::new(ArcSwapOption::empty()), budget })
    }
    /// The budget shared with the connector and every generation snapshot.
    pub(crate) fn budget(&self) -> &Arc<GatewayAdmissionBudget> {
        &self.budget
    }
    pub(crate) fn acquire_connection(
        &self,
        now: UnixInstant,
    ) -> Result<GatewayConnectionLease, GatewayAdmissionError> {
        let snapshot = self.current.load_full().ok_or(GatewayAdmissionError::Unavailable)?;
        if !snapshot.certified_key.is_usable_at(now) {
            return Err(GatewayAdmissionError::CertifiedKeyUnusable);
        }
        let public_connection = self.budget.try_public_connection()?;
        let generation = snapshot.retention.try_connection()?;
        Ok(GatewayConnectionLease {
            snapshot,
            generation,
            public_connection,
            requests_started: AtomicU32::new(0),
            request_active: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// One admitted public connection; retains one whole Current snapshot.
/// Neither lease implements `Clone` or `Default`.
pub(crate) struct GatewayConnectionLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationConnectionPermit,
    public_connection: GatewayPublicConnectionPermit,
    requests_started: AtomicU32,
    request_active: Arc<AtomicBool>,
}

impl GatewayConnectionLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot {
        &self.snapshot
    }
    pub(crate) fn try_request(&self) -> Result<GatewayRequestLease, GatewayAdmissionError> {
        if self
            .request_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            todo!("SCAFFOLD: GatewayConnectionLease::try_request refusal arm (request in flight)");
        }
        let ceiling = self.snapshot.budget.limits.max_requests_per_connection().get();
        let started =
            self.requests_started.fetch_update(Ordering::AcqRel, Ordering::Acquire, |started| {
                (started < ceiling).then_some(started + 1)
            });
        let Ok(previous) = started else {
            self.request_active.store(false, Ordering::Release);
            todo!("SCAFFOLD: GatewayConnectionLease::try_request refusal arm (request limit)");
        };
        let release = || {
            self.requests_started.fetch_sub(1, Ordering::AcqRel);
            self.request_active.store(false, Ordering::Release);
        };
        let global = match self.snapshot.budget.try_request() {
            Ok(global) => global,
            Err(error) => {
                release();
                return Err(error);
            }
        };
        let generation = match self.snapshot.retention.try_request() {
            Ok(generation) => generation,
            Err(error) => {
                drop(global);
                release();
                return Err(error);
            }
        };
        Ok(GatewayRequestLease {
            snapshot: Arc::clone(&self.snapshot),
            generation,
            global,
            request_active: Arc::clone(&self.request_active),
            close_after_response: previous + 1 == ceiling,
        })
    }
}

pub(crate) struct GatewayRequestLease {
    snapshot: Arc<GatewayApplicationSnapshot>,
    generation: GatewayGenerationRequestPermit,
    global: GatewayGlobalRequestPermit,
    request_active: Arc<AtomicBool>,
    close_after_response: bool,
}

impl Drop for GatewayRequestLease {
    /// Reset the in-flight flag first; the permits then release in field order,
    /// each notifying the retention owner after the reset.
    fn drop(&mut self) {
        self.request_active.store(false, Ordering::Release);
    }
}

impl GatewayRequestLease {
    pub(crate) fn snapshot(&self) -> &GatewayApplicationSnapshot {
        &self.snapshot
    }
    pub(crate) const fn close_after_response(&self) -> bool {
        self.close_after_response
    }
}

#[derive(Debug)]
pub(crate) enum GatewayAdmissionError {
    Unavailable,
    CertifiedKeyUnusable,
    ConnectionLimit,
    ConnectLimit,
    RequestLimit,
}

pub(crate) struct GatewayDrainTimeout {
    pub generation: GatewayApplicationGenerationId,
    pub connections: u32,
    pub requests: u32,
}

#[derive(Debug)]
pub(crate) enum GatewayApplicationError {
    CertifiedKeyReferenceMismatch,
    CertifiedKeyHostnameMismatch,
    Frontend(ServiceFrontendResolveError),
    Identity(GatewayIdentityLifecycleError),
    UpstreamProbe(GatewayUpstreamConnectError),
    Bind(std::io::ErrorKind),
    WatchGap,
    Superseded,
}

// ---------------------------------------------------------------------------
// Bounded cleanup ledger (R-5, DG-11: reserve before `open_upstream`, bind after)
// ---------------------------------------------------------------------------

/// One slot of the ledger: reserved and not yet bound, or the record of a
/// registered intent.
enum LedgerSlot {
    Reserved,
    Bound(GatewayCleanupRecord),
}

pub(crate) struct GatewayCleanupLedger {
    capacity: NonZeroU32,
    saturation_count: AtomicU64,
    slots: Mutex<LedgerSlots>,
}

#[derive(Default)]
struct LedgerSlots {
    next_slot: u64,
    by_slot: BTreeMap<u64, LedgerSlot>,
}

impl LedgerSlots {
    /// Takes one slot while fewer than `capacity` are held.
    fn reserve_below(&mut self, capacity: NonZeroU32) -> Option<u64> {
        let held = u32::try_from(self.by_slot.len()).unwrap_or(u32::MAX);
        if held >= capacity.get() {
            return None;
        }
        let slot = self.next_slot;
        self.next_slot = self.next_slot.saturating_add(1);
        self.by_slot.insert(slot, LedgerSlot::Reserved);
        Some(slot)
    }
}

pub(crate) struct GatewayCleanupRecord {
    intent: GatewayConnectIntent,
    connect_intent_failed: bool,
    receipt_failed: bool,
}

pub(crate) struct GatewayCleanupReservation {
    ledger: Arc<GatewayCleanupLedger>,
    slot: u64,
    bound: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GatewayCleanupLedgerFull {
    pub capacity: NonZeroU32,
}

impl GatewayCleanupLedger {
    pub(crate) fn new(capacity: NonZeroU32) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            saturation_count: AtomicU64::new(0),
            slots: Mutex::new(LedgerSlots::default()),
        })
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, LedgerSlots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes one slot before `open_upstream`; the slot is bound to the intent
    /// afterwards. Dropping a reservation that was never bound releases it.
    /// A full ledger counts one saturation and refuses.
    pub(crate) fn reserve(
        self: &Arc<Self>,
    ) -> Result<GatewayCleanupReservation, GatewayCleanupLedgerFull> {
        let Some(slot) = self.slots().reserve_below(self.capacity) else {
            self.saturation_count
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    Some(count.saturating_add(1))
                })
                .unwrap_or_else(|_| unreachable!("the update closure always returns Some"));
            return Err(GatewayCleanupLedgerFull { capacity: self.capacity });
        };
        Ok(GatewayCleanupReservation { ledger: Arc::clone(self), slot, bound: false })
    }

    /// Cleans the exact intent of every retained record. The mutex guard is
    /// dropped before any cleanup call; only successfully cleaned records are
    /// removed.
    pub(crate) fn retry_all(
        &self,
        cleanup: &dyn GatewayConnectCleanup,
    ) -> Vec<GatewayCleanupFailure> {
        let snapshot: Vec<(u64, GatewayConnectIntent, bool, bool)> = self
            .slots()
            .by_slot
            .iter()
            .filter_map(|(slot, entry)| match entry {
                LedgerSlot::Bound(record) => Some((
                    *slot,
                    record.intent.clone(),
                    record.connect_intent_failed,
                    record.receipt_failed,
                )),
                LedgerSlot::Reserved => None,
            })
            .collect();
        let (mut intent_failures, mut receipt_failures) = (0_u32, 0_u32);
        let mut cleaned = Vec::with_capacity(snapshot.len());
        for (slot, intent, intent_failed, receipt_failed) in snapshot {
            if cleanup.cleanup(&intent).is_ok() {
                cleaned.push(slot);
            } else {
                intent_failures += u32::from(intent_failed || !receipt_failed);
                receipt_failures += u32::from(receipt_failed);
            }
        }
        let mut slots = self.slots();
        for slot in cleaned {
            slots.by_slot.remove(&slot);
        }
        drop(slots);
        let mut failures = Vec::with_capacity(3);
        if intent_failures > 0 {
            failures.push(GatewayCleanupFailure::ConnectIntent { count: intent_failures });
        }
        if receipt_failures > 0 {
            failures.push(GatewayCleanupFailure::Receipt { count: receipt_failures });
        }
        let attempts = self.saturation_count.load(Ordering::Acquire);
        if attempts > 0 {
            failures.push(GatewayCleanupFailure::LedgerSaturated {
                attempts,
                capacity: self.capacity.get(),
            });
        }
        failures
    }

    pub(crate) fn len(&self) -> u32 {
        u32::try_from(self.slots().by_slot.len()).unwrap_or(u32::MAX)
    }
}

impl GatewayCleanupReservation {
    /// Binds the reserved slot to the intent `open_upstream` registered.
    pub(crate) fn bind(&mut self, intent: GatewayConnectIntent) {
        self.ledger.slots().by_slot.insert(
            self.slot,
            LedgerSlot::Bound(GatewayCleanupRecord {
                intent,
                connect_intent_failed: false,
                receipt_failed: false,
            }),
        );
        self.bound = true;
    }
    /// Marks the bound record's connect-intent key as failed to clean.
    pub(crate) fn record_intent_failure(&self) {
        self.mark_bound(|record| record.connect_intent_failed = true);
    }
    /// Marks the bound record's receipt key as failed to clean.
    pub(crate) fn record_receipt_failure(&self) {
        self.mark_bound(|record| record.receipt_failed = true);
    }
    fn mark_bound(&self, mark: impl FnOnce(&mut GatewayCleanupRecord)) {
        if let Some(LedgerSlot::Bound(record)) = self.ledger.slots().by_slot.get_mut(&self.slot) {
            mark(record);
        }
    }
    /// Removes the record: both keys are known clean.
    pub(crate) fn complete(mut self) {
        self.ledger.slots().by_slot.remove(&self.slot);
        self.bound = true;
    }
}

impl Drop for GatewayCleanupReservation {
    fn drop(&mut self) {
        if !self.bound {
            self.ledger.slots().by_slot.remove(&self.slot);
        }
    }
}
