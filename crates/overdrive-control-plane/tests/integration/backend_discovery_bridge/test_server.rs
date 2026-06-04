//! `TestServer` fixture for the walking-skeleton (S-BDB-01) Tier 3
//! e2e test.
//!
//! Per `backend-discovery-bridge-service-reachability/design/architecture.md`
//! § 6.2 (D7 / Atlas Q1 / DWD-09):
//!
//! - Lives entirely under `tests/` — NOT in `src/`. No production
//!   surface widening; the fixture is reachable only from sibling
//!   integration tests in this directory.
//! - Drives `submit_workload` through the **real HTTPS client**
//!   (reqwest + rustls), not via direct in-process handler call.
//!   The whole point of the walking-skeleton is to prove the entire
//!   driving-port stack works end-to-end: TLS handshake, axum
//!   routing, JSON decode, handler body, allocator round-trip,
//!   response encode. A direct internal call would defeat that.
//! - Binds an OS-assigned port (`127.0.0.1:0`) so concurrent tests do
//!   not collide.
//! - Owns the `EbpfDataplane` itself (constructed BEFORE
//!   `run_server`, injected via `dataplane_override`) so the test
//!   can call the cfg-gated `backend_map_entries()` /
//!   `service_map_contains()` accessors on the SAME dataplane the
//!   bridge / hydrator are writing into. The production boot path
//!   would otherwise own the only handle.
//! - Cleans up via `Drop` — the server task is cancelled and the
//!   `tempfile::TempDir` is unlinked. XDP / bpffs cleanup belongs to
//!   the per-test veth fixture in `walking_skeleton.rs`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    dead_code,
    reason = "Test fixture; failures must panic with informative messages"
)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{ServiceV1, WorkloadIntent};
use overdrive_core::api::submit::{ServiceSpecInput, SubmitSpecInput};
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::EbpfDataplane;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

/// In-process production server + a reqwest client trusting the
/// server's ephemeral CA + a back-channel `Arc<EbpfDataplane>` for
/// in-test `BACKEND_MAP` / `SERVICE_MAP` inspection. Drop tears down
/// the server task; the kernel-side resources detach via the
/// inner `EbpfDataplane::Drop` impl once the inner Arc's last
/// reference is released (which happens when `AppState` drops at
/// shutdown end).
pub struct TestServer {
    handle: Option<ServerHandle>,
    bound: SocketAddr,
    client: reqwest::Client,
    /// Production dataplane handle — same `Arc` `AppState`'s
    /// `dataplane: Arc<dyn Dataplane>` field stores. Used by the
    /// walking-skeleton to call the cfg-gated `backend_map_entries()`
    /// / `service_map_contains()` accessors.
    dataplane: Arc<EbpfDataplane>,
    /// Production observation-store handle — same `Arc` `AppState`'s
    /// `obs: Arc<dyn ObservationStore>` field stores. Used by the
    /// walking-skeleton to poll `alloc_status_rows()` without
    /// opening a second redb handle (which would race the writer
    /// on the file lock).
    obs: Arc<dyn ObservationStore>,
    /// Retained so the redb / trust-triple files outlive every call;
    /// dropped after `handle` so the server task releases its file
    /// locks before the tempdir is unlinked.
    _tmp: TempDir,
    /// The data dir under the tempdir.
    data_dir: PathBuf,
}

impl TestServer {
    /// Spawn a production server wired against `dataplane_config`
    /// (real `EbpfDataplane`, real XDP attach, real bpffs pin).
    ///
    /// The dataplane is constructed BEFORE `run_server` and the
    /// `Arc<EbpfDataplane>` is shared (a) via
    /// `ServerConfig::dataplane_override` so the production boot
    /// path adopts the same handle and (b) retained here as
    /// `self.dataplane` so the test can call the cfg-gated
    /// inspection accessors.
    pub async fn serve_with_dataplane(dataplane_config: DataplaneConfig, pin_dir: PathBuf) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let cfg_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("mkdir data");
        std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");

        // Construct the EbpfDataplane up-front. The production boot
        // path's Earned-Trust probe runs only when `dataplane_override`
        // is None; we own the override, so we run the probe here in
        // the same "wire then probe then use" order architecture.md §
        // 5.4 mandates.
        let ebpf = EbpfDataplane::new_with_pin_dir(
            &dataplane_config.client_iface,
            &dataplane_config.backend_iface,
            &pin_dir,
            std::path::Path::new("/sys/fs/cgroup"),
        )
        .expect("EbpfDataplane::new_with_pin_dir");
        ebpf.probe().await.expect("EbpfDataplane probe");
        let dataplane = Arc::new(ebpf);

        // Construct the observation store up-front so we can retain
        // an `Arc<dyn ObservationStore>` handle the test reads
        // through. Without this, a back-door `LocalObservationStore::open`
        // collides on redb's exclusive file lock against the
        // production server's writer handle.
        let obs_path = data_dir.join("observation.redb");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(LocalObservationStore::open(&obs_path).expect("open LocalObservationStore"));

        // Production driver: ExecDriver rooted at /sys/fs/cgroup.
        let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new(
            std::path::PathBuf::from("/sys/fs/cgroup"),
            Arc::new(overdrive_host::SystemClock),
            Arc::new(overdrive_host::RealCgroupFs::new()),
        ));

        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("parse bind addr"),
            data_dir: data_dir.clone(),
            operator_config_dir: cfg_dir.clone(),
            dataplane: Some(dataplane_config),
            dataplane_pin_dir: Some(pin_dir),
            dataplane_override: Some(
                dataplane.clone() as Arc<dyn overdrive_core::traits::dataplane::Dataplane>
            ),
            // production: tick_cadence/clock default to 100ms +
            // SystemClock — the walking-skeleton runs real wall-clock
            // because it asserts on real kernel side effects.
            ..Default::default()
        };

        let handle = run_server_with_obs_and_driver(config, obs.clone(), driver)
            .await
            .expect("run_server_with_obs_and_driver");
        let bound = handle.local_addr().await.expect("bound addr");
        let ca_pem = read_ca_from_trust_triple(&cfg_dir);
        let client = client_trusting(&ca_pem);

        Self { handle: Some(handle), bound, client, dataplane, obs, _tmp: tmp, data_dir }
    }

    /// Issue `POST /v1/jobs` through the real HTTPS driving port.
    /// Returns the decoded `SubmitWorkloadResponse` carrying the
    /// allocator-issued VIP (for Service kinds).
    pub async fn submit_workload(&self, spec: SubmitSpecInput) -> SubmitWorkloadResponse {
        let url = format!("https://localhost:{}/v1/jobs", self.bound.port());
        let resp = self
            .client
            .post(&url)
            .json(&SubmitWorkloadRequest { spec })
            .send()
            .await
            .expect("submit_workload: POST /v1/jobs");
        let status = resp.status();
        let body_bytes = resp.bytes().await.expect("read response body");
        assert!(
            status.is_success(),
            "submit_workload non-success: status={status} body={}",
            String::from_utf8_lossy(&body_bytes),
        );
        serde_json::from_slice::<SubmitWorkloadResponse>(&body_bytes)
            .expect("decode SubmitWorkloadResponse from /v1/jobs body")
    }

    /// The production observation-store handle the server is using.
    /// Used by the walking-skeleton to poll `alloc_status_rows()`
    /// without racing the writer on redb's exclusive file lock.
    #[must_use]
    pub fn obs(&self) -> Arc<dyn ObservationStore> {
        Arc::clone(&self.obs)
    }

    /// The production dataplane handle the server is using.
    /// Used by the walking-skeleton to call the cfg-gated
    /// `backend_map_entries()` / `service_map_contains()` accessors
    /// on the SAME dataplane the bridge / hydrator are writing into.
    #[must_use]
    pub fn dataplane(&self) -> Arc<EbpfDataplane> {
        Arc::clone(&self.dataplane)
    }

    /// Storage root for the server's redb files. Useful for tests
    /// that need a back-door observation read (a fresh
    /// `LocalObservationStore::open` against
    /// `<data_dir>/observation.redb`).
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    /// Bound socket address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.bound
    }

    /// Trigger graceful shutdown — drains in-flight requests, stops
    /// the convergence loop, and drops the production `AppState`
    /// (which releases the second `Arc<EbpfDataplane>` clone). After
    /// this returns the inner dataplane Drop runs and XDP detaches +
    /// bpffs pin unlinks. Idempotent — second call is a no-op.
    pub async fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown(Duration::from_secs(2)).await;
        }
        // tmp drops at end of fn scope
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Best-effort: if the test forgot to call `shutdown().await`,
        // try to drain via a fresh runtime handle. `block_in_place`
        // requires the multi-thread flavour; on the single-thread
        // flavour this branch is skipped and the server task is
        // simply abandoned (kernel resources still detach via the
        // EbpfDataplane Arc's eventual Drop).
        if let Some(handle) = self.handle.take()
            && let Ok(handle_rt) = tokio::runtime::Handle::try_current()
        {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tokio::task::block_in_place(|| {
                    handle_rt.block_on(handle.shutdown(Duration::from_secs(2)));
                });
            }));
        }
    }
}

// ----------------------------------------------------------------------------
// Helpers — extracted to module scope so the test body stays terse.
// ----------------------------------------------------------------------------

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("read trust triple at {}: {e}", config_path.display()));
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|c| c.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

/// Local re-derivation of the `spec_digest` for a Service spec via
/// the same path the production `submit_workload` handler takes.
/// Returns the lowercase-hex digest (matching
/// `SubmitWorkloadResponse.spec_digest`).
#[must_use]
pub fn service_spec_digest_hex(spec: ServiceSpecInput) -> String {
    let service = ServiceV1::from_submit(spec).expect("Service spec must validate");
    let intent = WorkloadIntent::Service(service);
    intent.spec_digest().expect("spec_digest").to_string()
}

/// Poll a future until it yields `Some` or the wall-clock budget
/// expires. Cadence is the wall-clock pause between attempts.
///
/// Used by the walking-skeleton at three sites:
/// 1. Wait for `AllocState::Running`.
/// 2. Wait for `BACKEND_MAP` / `SERVICE_MAP` population.
/// 3. Wait for the TCP round-trip through the VIP to succeed.
///
/// Returns the inner `Some(T)` on success or `None` on timeout.
pub async fn poll_until<F, Fut, T>(budget: Duration, cadence: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + budget;
    loop {
        if let Some(v) = probe().await {
            return Some(v);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(cadence).await;
    }
}
