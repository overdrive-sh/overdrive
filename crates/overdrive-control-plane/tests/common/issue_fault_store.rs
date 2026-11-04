//! A delegating `ObservationStore` whose faults are scoped to one write or one
//! read, every other method delegating to the node's own store, and which
//! counts the two status point reads the status handler can issue.
//!
//! The faults enter through the existing driven port (the observation store the
//! composition was built over), never through a production seam:
//!
//! * `fail_issued_certificate_writes` fails only the `issued_certificates`
//!   audit write, so a status or failure-record write still succeeds (the
//!   `Audit` stimulus);
//! * `fail_failure_record` fails only the workload failure-record operation
//!   (the unrecorded arm);
//! * `fail_issue_status_reads` fails only the workload failure-row point read;
//! * `fail_gateway_status_writes` / `fail_gateway_status_reads` fail only the
//!   gateway status row write / point read; `fail_certified_key_status_reads`
//!   fails only the certified-key status point read;
//! * `status_point_read_count` counts the two point reads
//!   `GET /v1/gateway/status` can issue (nothing else, so concurrent
//!   reconciler or interest-router reads never perturb it);
//!   `gateway_status_writes` keeps every gateway status row offered to
//!   `write`, accepted or refused.
//!
//! `#[path]`-included by each test binary that needs it.

#![allow(dead_code, reason = "each including test binary uses a subset of the faults")]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use overdrive_core::ca::issue_failure::SvidIssueFailure;
use overdrive_core::id::{AllocationId, NodeId, ServiceId};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, PublicCertifiedKeyId, PublicCertifiedKeyStatusRowV1,
};
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocStatusRow, LagAwareSubscription, NodeHealthRow,
    ObservationStore, ObservationStoreError, ObservationWrite, ReconcileConflictRow,
    ServiceBackendRow, ServiceHydrationResultRow, TransitionSource, WorkloadSvidIssueStatusRowV1,
};

/// The text every injected fault carries; no operator surface may contain it.
pub const ISSUE_FAULT_MARKER: &str = "issue-fault-store-marker";

pub struct IssueFaultStore {
    pub inner: Arc<dyn ObservationStore>,
    pub fail_issued_certificate_writes: AtomicBool,
    pub fail_failure_record: AtomicBool,
    pub fail_issue_status_reads: AtomicBool,
    pub fail_gateway_status_writes: AtomicBool,
    pub fail_gateway_status_reads: AtomicBool,
    pub fail_certified_key_status_reads: AtomicBool,
    status_point_reads: AtomicUsize,
    pub gateway_status_writes: Mutex<Vec<GatewayApplicationStatusRowV1>>,
}

impl IssueFaultStore {
    #[must_use]
    pub fn over(inner: Arc<dyn ObservationStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            fail_issued_certificate_writes: AtomicBool::new(false),
            fail_failure_record: AtomicBool::new(false),
            fail_issue_status_reads: AtomicBool::new(false),
            fail_gateway_status_writes: AtomicBool::new(false),
            fail_gateway_status_reads: AtomicBool::new(false),
            fail_certified_key_status_reads: AtomicBool::new(false),
            status_point_reads: AtomicUsize::new(0),
            gateway_status_writes: Mutex::new(Vec::new()),
        })
    }

    /// Every gateway-status and certified-key-status point read served so far.
    #[must_use]
    pub fn status_point_read_count(&self) -> usize {
        self.status_point_reads.load(Ordering::SeqCst)
    }

    /// Every gateway status row offered to `write`, in order.
    #[must_use]
    pub fn offered_gateway_status_rows(&self) -> Vec<GatewayApplicationStatusRowV1> {
        self.gateway_status_writes.lock().expect("offered status rows").clone()
    }
}

fn fault() -> ObservationStoreError {
    ObservationStoreError::Unreachable { peer: ISSUE_FAULT_MARKER.to_owned() }
}

#[async_trait::async_trait]
impl ObservationStore for IssueFaultStore {
    async fn public_certified_key_status_row(
        &self,
        id: &PublicCertifiedKeyId,
    ) -> Result<Option<PublicCertifiedKeyStatusRowV1>, ObservationStoreError> {
        self.status_point_reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_certified_key_status_reads.load(Ordering::SeqCst) {
            return Err(fault());
        }
        self.inner.public_certified_key_status_row(id).await
    }

    async fn gateway_application_status_row(
        &self,
        node_id: &NodeId,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, ObservationStoreError> {
        self.status_point_reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_gateway_status_reads.load(Ordering::SeqCst) {
            return Err(fault());
        }
        self.inner.gateway_application_status_row(node_id).await
    }

    async fn record_workload_svid_issue_failure(
        &self,
        alloc_id: &AllocationId,
        node_id: &NodeId,
        failure: SvidIssueFailure,
    ) -> Result<(), ObservationStoreError> {
        if self.fail_failure_record.load(Ordering::SeqCst) {
            return Err(fault());
        }
        self.inner.record_workload_svid_issue_failure(alloc_id, node_id, failure).await
    }

    async fn workload_svid_issue_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<WorkloadSvidIssueStatusRowV1>, ObservationStoreError> {
        if self.fail_issue_status_reads.load(Ordering::SeqCst) {
            return Err(fault());
        }
        self.inner.workload_svid_issue_status_row(alloc_id).await
    }

    async fn write(&self, row: ObservationWrite) -> Result<(), ObservationStoreError> {
        match &row {
            ObservationWrite::IssuedCertificate(_)
                if self.fail_issued_certificate_writes.load(Ordering::SeqCst) =>
            {
                return Err(fault());
            }
            ObservationWrite::GatewayApplicationStatus(status) => {
                self.gateway_status_writes
                    .lock()
                    .expect("offered status rows")
                    .push(status.clone());
                if self.fail_gateway_status_writes.load(Ordering::SeqCst) {
                    return Err(fault());
                }
            }
            _ => {}
        }
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.write_alloc_lifecycle(current, source).await
    }

    async fn alloc_lifecycle_occurrences(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.alloc_lifecycle_occurrences(alloc_id).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        self.inner.subscribe_all_events().await
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_rows().await
    }

    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_row(alloc_id).await
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        self.inner.node_health_rows().await
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<
        Vec<overdrive_core::ca::issued_certificate_row::IssuedCertificateRow>,
        ObservationStoreError,
    > {
        self.inner.issued_certificate_rows().await
    }

    async fn next_issuance_ordinal(
        &self,
    ) -> Result<overdrive_core::id::IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }

    async fn write_probe_result(
        &self,
        row: overdrive_core::observation::ProbeResultRow,
    ) -> Result<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }

    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<overdrive_core::observation::ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }

    async fn workflow_terminal_rows(
        &self,
    ) -> Result<
        Vec<(overdrive_core::id::CorrelationKey, overdrive_core::workflow::WorkflowStatus)>,
        ObservationStoreError,
    > {
        self.inner.workflow_terminal_rows().await
    }

    async fn workflow_signal(
        &self,
        key: &overdrive_core::workflow::SignalKey,
    ) -> Result<Option<overdrive_core::workflow::SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }

    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        self.inner.service_hydration_results_rows(service_id).await
    }

    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }

    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
        self.inner.reconcile_conflict_rows(service_id).await
    }
}
