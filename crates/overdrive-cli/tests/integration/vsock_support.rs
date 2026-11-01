//! Shared support of the vsock native-metal lanes driven through
//! `overdrive serve` + `overdrive deploy` (GH #295): the walking skeleton
//! (`vsock_walking_skeleton`, P6-S13), the native fault matrix
//! (`vsock_fault_matrix`, P6-S14) and the density and fail-closed cells
//! (`vsock_density`, P6-S15 / V-9).
//!
//! **Production composition, no test wiring.** Every lane boots the
//! in-process `overdrive serve` through `serve_fixture::serve` (`DataplaneChoice::Host`) — the same
//! `run_server` composition `overdrive serve` runs, the real `EbpfDataplane`
//! and the mTLS and guest-network owners composed, only the KEK injected
//! (`SimKek`; the kernel keyring refuses a cold boot) — and submits VM
//! workloads through the public deploy handler. Nothing here installs a
//! rule, binds a listener, programs a map or supplies an address that
//! `run_server` does not (CLAUDE.md § "Build vertical slices through
//! production entry points").
//!
//! **Lane setup.** Run `cargo xtask guest-kit-prepare` (under `cargo xtask
//! metal run --`) before these lanes: it builds `ovd_fwd.ko` for the running
//! kernel and the static guest program. `GuestKit::provision` builds
//! nothing; it refuses `LaneNotPrepared`, naming that command, when they are
//! missing.
//!
//! **Guests.** Each VM spec names the guest kit's kernel and a guest-kit root
//! filesystem (`overdrive_testing::guest_kit`, EVMOD-GUEST-KIT: the kit's
//! `overdrive-init` and its `ovd_fwd.ko`); the operator command is the guest
//! application `overdrive-guest-peer` (`overdrive_testing::guest_app`), an
//! ordinary identity-unaware program that opens plain TCP and UDP sockets.
//!
//! **The far side.** A `RemotePeer`: TCP and UDP servers in their own
//! network namespace on a leased test `/24`, off-host from the node's point
//! of view; the source address of every connection it accepts is the
//! node's veth address, which proves the guest's bytes crossed the host's
//! forwarder.
//!
//! **Observation.** Operator surfaces (`overdrive workload describe`), the
//! remote peer's own sockets, the guest console log the VMM writes for the
//! allocation (the guest application's stdout), and the kernel's process
//! table (the VMM process, its `--vsock` argument, `/dev/vhost-vsock`
//! holders).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(dead_code, reason = "shared by three kvm-gated lane modules")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, AllocStatusRowBody};
use overdrive_control_plane::ca_boot;
use overdrive_control_plane::node_adapters::{CaAdapters, IntentStoreAdapter};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::ca::{SvidMaterial, SvidRequest, TrustBundle};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::vm::config::VmRunDir;
use overdrive_core::{SpiffeId, wall_clock::UnixInstant};
use overdrive_sim::adapters::SimKek;
use overdrive_testing::guest_app::{
    GUEST_PEER_GUEST_PATH, V6_APP_PORT, V6_CLIENT_ADMISSION_PORT, V6_HTTP_PROBE_PORT,
    V6_POPULATION_BASE_PORT, V6_TCP_PROBE_PORT,
};
use overdrive_testing::guest_kit::{GuestImage, GuestImageSpec, GuestKit};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::verify_server_cert_signed_by_trust_anchor;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use tempfile::TempDir;
use tracing_subscriber::EnvFilter;
use x509_parser::prelude::FromDer as _;

use super::vm_walking_skeleton::{
    config_path, server_tmp_on_staging_root, shared_staging_root, write_toml,
};

/// Where the VM driver keeps each allocation's run directory (and the
/// console log the VMM writes there).
const VM_RUN_ROOT: &str = "/run/overdrive/vm";

fn retained_consoles() -> &'static Mutex<BTreeMap<String, Vec<u8>>> {
    static CONSOLES: OnceLock<Mutex<BTreeMap<String, Vec<u8>>>> = OnceLock::new();
    CONSOLES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn all_console_diagnostics() -> String {
    let consoles = retained_consoles().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut output = String::new();
    for (alloc, bytes) in consoles.iter() {
        writeln!(&mut output, "--- allocation={alloc} ---").expect("write console heading");
        output.push_str(&String::from_utf8_lossy(bytes));
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    drop(consoles);
    output
}

fn init_serve_trace_output() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let filter = EnvFilter::new(
            "info,overdrive_control_plane=info,overdrive_worker=debug,overdrive_dataplane=debug",
        );
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(true)
            .compact()
            .try_init();
    });
}

/// Test-local read observer for the production console file. The natural-exit
/// watcher removes the run directory before it emits the `ExitEvent` that
/// makes the terminal row observable, so a post-terminal path read is
/// necessarily empty. Retain only bytes that the existing console file
/// exposed before accepted cleanup; production still owns and removes the
/// file on its original schedule.
struct ConsoleCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ConsoleCapture {
    fn start() -> Self {
        retained_consoles().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("vsock-console-capture".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    if let Ok(entries) = std::fs::read_dir(VM_RUN_ROOT) {
                        for entry in entries.flatten() {
                            let alloc = entry.file_name().to_string_lossy().into_owned();
                            let Ok(bytes) = std::fs::read(entry.path().join("console.log")) else {
                                continue;
                            };
                            if bytes.is_empty() {
                                continue;
                            }
                            let mut consoles = retained_consoles()
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            let retained = consoles.entry(alloc).or_default();
                            if bytes.len() >= retained.len() {
                                *retained = bytes;
                            }
                            drop(consoles);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .expect("spawn the test-local console capture");
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for ConsoleCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone)]
struct RemoteIdentity {
    root: String,
    intermediate: String,
    leaf: String,
    key: String,
}

pub(super) struct V6ClientSpec<'a> {
    pub(super) id: &'a str,
    pub(super) peer_name: &'a str,
    pub(super) peer_vip: Ipv4Addr,
    pub(super) off_tcp: SocketAddrV4,
    pub(super) off_udp: SocketAddrV4,
    pub(super) population: u16,
    pub(super) tag: &'a str,
}

async fn prepare_remote_identity(data_dir: &Path) -> RemoteIdentity {
    let store = IntentStoreAdapter::open(data_dir).expect("open V-6 intent store for CA seed");
    let intent: Arc<dyn IntentStore> = Arc::clone(store.store()) as Arc<dyn IntentStore>;
    let CaAdapters { factory, root_key_codec } = CaAdapters::host();
    let ca = factory.build(Arc::new(overdrive_host::OsEntropy));
    let kek = SimKek::for_boot();
    let kek_id = ca_boot::root_kek_id();
    let node = NodeId::new("local").expect("production local node id");
    ca_boot::boot_ca(ca.as_ref(), &kek, &kek_id, root_key_codec.as_ref(), &intent, store.path())
        .await
        .expect("seed the production workload root CA");
    ca_boot::bootstrap_node_intermediate(
        ca.as_ref(),
        &node,
        &intent,
        &kek,
        &kek_id,
        root_key_codec.as_ref(),
        store.path(),
    )
    .await
    .expect("seed the production workload intermediate");
    let now =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("wall clock after epoch").as_secs();
    let request = SvidRequest::new(
        SpiffeId::new("spiffe://overdrive.local/workload/v6-remote/alloc/external")
            .expect("valid external V-6 SPIFFE id"),
        UnixInstant::from_unix_duration(Duration::from_secs(now.saturating_sub(60))),
        UnixInstant::from_unix_duration(Duration::from_secs(now + 3_600)),
    );
    let svid = ca.issue_svid(&request).expect("issue the remote V-6 fixture SVID");
    let bundle = ca.trust_bundle().expect("compose the workload trust bundle");
    let identity = RemoteIdentity::from_material(&svid, &bundle);
    drop((intent, store, ca));
    identity
}

impl RemoteIdentity {
    fn from_material(svid: &SvidMaterial, bundle: &TrustBundle) -> Self {
        Self {
            root: bundle.root_anchor().as_pem().to_owned(),
            intermediate: bundle
                .intermediate_chain()
                .expect("the production node intermediate exists")
                .as_pem()
                .to_owned(),
            leaf: svid.cert_pem().as_pem().to_owned(),
            key: svid.leaf_key().as_pem().to_owned(),
        }
    }
}

/// Guest RAM of every lane VM (the kit's stock kernel boots in it).
pub(super) const LANE_VM_MEMORY_BYTES: u64 = 128 * 1024 * 1024;

/// One booted `serve` with the guest kit and a workload image.
pub(super) struct Lane {
    pub(super) kit: GuestKit,
    pub(super) image: GuestImage,
    handle: Option<ServeHandle>,
    pub(super) server_tmp: TempDir,
    remote_identity: RemoteIdentity,
    _console_capture: ConsoleCapture,
}

impl Lane {
    /// Provisions the guest kit, boots `serve` in its production
    /// composition, and builds the workload image `name` next to its data
    /// directory (same filesystem: the VM driver reflinks the rootfs master
    /// into the data directory).
    pub(super) async fn boot(name: &str) -> Self {
        init_serve_trace_output();
        let kit = GuestKit::provision(&shared_staging_root())
            .expect("precondition: the guest kit provisions (kernel, ovd_fwd.ko, guest programs)");
        let server_tmp = server_tmp_on_staging_root();
        let data_dir = server_tmp.path().join("data");
        let config_dir = server_tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create V-6 server data dir");
        std::fs::create_dir_all(&config_dir).expect("create V-6 operator config dir");
        let remote_identity = prepare_remote_identity(&data_dir).await;
        let args = super::serve_fixture::serve_args(
            "127.0.0.1:0".parse::<SocketAddr>().expect("bind addr"),
            data_dir,
            config_dir,
        );
        let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Host)
            .await
            .expect("serve must start (host dataplane)");
        let image = kit
            .image(server_tmp.path(), name, &GuestImageSpec::workload())
            .expect("the workload image");
        let console_capture = ConsoleCapture::start();
        Self {
            kit,
            image,
            handle: Some(handle),
            server_tmp,
            remote_identity,
            _console_capture: console_capture,
        }
    }

    /// The operator config the deploy and describe handlers read.
    pub(super) fn cfg(&self) -> PathBuf {
        config_path(self.server_tmp.path())
    }

    /// Deploys a VM Job whose operator command is the guest application
    /// with `args`; returns the workload id.
    pub(super) async fn deploy_job(&self, id: &str, args: &[&str]) -> String {
        let args_toml = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
        let body = format!(
            "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [{args_toml}]\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
             memory_bytes = {LANE_VM_MEMORY_BYTES}\n",
            self.kit.kernel_path.display(),
            self.image.rootfs.display(),
        );
        let spec = write_toml(self.server_tmp.path(), &format!("{id}.toml"), &body);
        deploy(DeployArgs { spec, config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("deploy {id}: {e}"))
            .workload_id
    }

    /// Deploys the fault matrix's long-running `hold` workload as a Service.
    /// RESTART-SSOT finalizes a naturally crashed Job, while a Service below
    /// its restart ceiling places a successor. The declared listener is the
    /// Service admission shape only; the fault oracle observes the guest's
    /// outbound hold flow, so this fixture declares no health probes.
    pub(super) async fn deploy_hold_service(&self, id: &str, args: &[&str]) -> String {
        const ADMISSION_PORT: u16 = 8080;

        let args_toml = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
        let body = format!(
            "[service]\nid = \"{id}\"\nreplicas = 1\n\n[vm]\ncommand = \"{GUEST_PEER_GUEST_PATH}\"\n\
             args = [{args_toml}]\nkernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\n\
             cpu_milli = 500\nmemory_bytes = {LANE_VM_MEMORY_BYTES}\n\n[[listener]]\n\
             port = {ADMISSION_PORT}\nprotocol = \"tcp\"\n",
            self.kit.kernel_path.display(),
            self.image.rootfs.display(),
        );
        self.deploy_service_toml(id, &body).await
    }

    pub(super) async fn deploy_v6_peer(&self, id: &str, population: u16) -> String {
        let mut listeners = format!(
            "[[listener]]\nport = {V6_APP_PORT}\nprotocol = \"tcp\"\n\n\
             [[listener]]\nport = {V6_TCP_PROBE_PORT}\nprotocol = \"tcp\"\n\n\
             [[listener]]\nport = {V6_HTTP_PROBE_PORT}\nprotocol = \"tcp\"\n\n"
        );
        for index in 0..population {
            write!(
                &mut listeners,
                "[[listener]]\nport = {}\nprotocol = \"tcp\"\n\n",
                V6_POPULATION_BASE_PORT + index
            )
            .expect("write listener TOML to String");
        }
        let body = format!(
            "[service]\nid = \"{id}\"\nreplicas = 1\n\n[vm]\n\
             command = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [\"v6-peer\", \"{population}\"]\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
             memory_bytes = {LANE_VM_MEMORY_BYTES}\n\n{listeners}\
             [[health_check.readiness]]\ntype = \"tcp\"\nhost = \"0.0.0.0\"\n\
             port = {V6_TCP_PROBE_PORT}\ninterval_seconds = 1\ntimeout_seconds = 1\n\
             max_attempts = 30\nfailure_threshold = 1\nsuccess_threshold = 1\n\n\
             [[health_check.readiness]]\ntype = \"http\"\npath = \"/healthz\"\n\
             port = {V6_HTTP_PROBE_PORT}\ninterval_seconds = 1\ntimeout_seconds = 1\n\
             max_attempts = 30\nfailure_threshold = 1\nsuccess_threshold = 1\n",
            self.kit.kernel_path.display(),
            self.image.rootfs.display(),
        );
        self.deploy_service_toml(id, &body).await
    }

    pub(super) async fn deploy_v6_client(&self, spec: V6ClientSpec<'_>) -> String {
        let V6ClientSpec { id, peer_name, peer_vip, off_tcp, off_udp, population, tag } = spec;
        let body = format!(
            "[service]\nid = \"{id}\"\nreplicas = 1\n\n[vm]\n\
             command = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [\"v6-client\", \"{peer_name}\", \
             \"{peer_vip}\", \"{off_tcp}\", \"{off_udp}\", \"{population}\", \"{tag}\"]\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
             memory_bytes = {LANE_VM_MEMORY_BYTES}\n\n[[listener]]\n\
             port = {V6_CLIENT_ADMISSION_PORT}\nprotocol = \"tcp\"\n",
            self.kit.kernel_path.display(),
            self.image.rootfs.display(),
        );
        self.deploy_service_toml(id, &body).await
    }

    async fn deploy_service_toml(&self, id: &str, body: &str) -> String {
        let spec = write_toml(self.server_tmp.path(), &format!("{id}.toml"), body);
        deploy(DeployArgs { spec, config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("deploy Service {id}: {e}"))
            .workload_id
    }

    /// Stops a workload through the operator verb.
    pub(super) async fn stop(&self, id: &str) {
        stop(StopArgs { id: id.to_owned(), config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("stop {id}: {e}"));
    }

    /// The workload's allocation rows.
    pub(super) async fn rows(&self, id: &str) -> Vec<AllocStatusRowBody> {
        self.snapshot(id).await.rows
    }

    pub(super) async fn snapshot(&self, id: &str) -> AllocStatusResponse {
        describe(DescribeArgs { id: id.to_owned(), config_path: self.cfg() })
            .await
            .unwrap_or_else(|e| panic!("describe {id}: {e}"))
            .snapshot
    }

    /// Polls until `done` holds over the rows, or panics with the last rows.
    pub(super) async fn wait_rows(
        &self,
        id: &str,
        bound: Duration,
        what: &str,
        done: impl Fn(&[AllocStatusRowBody]) -> bool,
    ) -> Vec<AllocStatusRowBody> {
        let deadline = tokio::time::Instant::now() + bound;
        loop {
            let rows = self.rows(id).await;
            if done(&rows) {
                return rows;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workload {id}: {what} not reached within {bound:?}; rows: {rows:#?}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The first allocation of `id` once it is Running.
    pub(super) async fn running_alloc(&self, id: &str, bound: Duration) -> AllocationId {
        let rows = self
            .wait_rows(id, bound, "a Running allocation", |rows| {
                rows.iter().any(|r| r.state == AllocStateWire::Running)
            })
            .await;
        let row = rows.iter().find(|r| r.state == AllocStateWire::Running).expect("a Running row");
        AllocationId::new(&row.alloc_id).expect("a valid allocation id")
    }

    pub(super) async fn running_alloc_with_console(
        &self,
        id: &str,
        bound: Duration,
        done: impl Fn(&str) -> bool,
    ) -> (AllocationId, String) {
        let deadline = tokio::time::Instant::now() + bound;
        let mut last = Vec::new();
        loop {
            let rows = self.rows(id).await;
            last.clear();
            for row in rows.iter().filter(|row| row.state == AllocStateWire::Running) {
                let alloc = AllocationId::new(&row.alloc_id).expect("a valid allocation id");
                let console = console_of(&alloc);
                if done(&console) {
                    return (alloc, console);
                }
                last.push((alloc, console));
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workload {id}: no Running allocation reached the console outcome within \
                 {bound:?}; running consoles: {last:#?}; rows: {rows:#?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub(super) async fn wait_console(
        &self,
        alloc: &AllocationId,
        bound: Duration,
        done: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        wait_console(alloc, bound, done).await.map_err(|console| {
            format!(
                "allocation={alloc}\n{console}\nall allocation consoles:\n{}",
                all_console_diagnostics()
            )
        })
    }

    /// `serve` dies without a graceful drain (the serve lifetime port's
    /// killed mode): every in-process task, socket and descriptor of the
    /// owner is released, as the kernel releases them at a `SIGKILL`, and
    /// the VMMs survive.
    pub(super) async fn kill_serve(&mut self) {
        let handle = self.handle.take().expect("serve is running");
        super::serve_lifetime_support::kill_serve_owner(handle).await.expect("the killed mode");
    }

    /// Boots `serve` again on the same data and config directories (the
    /// supervisor's restart after a crash).
    pub(super) async fn reboot_serve(&mut self) {
        assert!(self.handle.is_none(), "serve is down");
        let args = super::serve_fixture::serve_args(
            "127.0.0.1:0".parse::<SocketAddr>().expect("bind addr"),
            self.server_tmp.path().join("data"),
            self.server_tmp.path().join("conf"),
        );
        let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Host)
            .await
            .expect("serve boots again on the same directories");
        self.handle = Some(handle);
    }

    /// Graceful shutdown.
    pub(super) async fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown().await.expect("clean shutdown");
        }
    }

    pub(super) fn remote_mtls_round_trip(&self, namespace: &str, target: SocketAddrV4) {
        let identity = self.remote_identity.clone();
        let namespace = namespace.to_owned();
        std::thread::spawn(move || {
            let ns = std::fs::File::open(format!("/var/run/netns/{namespace}"))
                .expect("open the route-disjoint peer namespace");
            // SAFETY: `ns` is a live network-namespace descriptor and setns
            // changes only this fixture thread.
            assert_eq!(unsafe { libc::setns(ns.as_raw_fd(), libc::CLONE_NEWNET) }, 0);
            let mut roots = rustls::RootCertStore::empty();
            let mut root = std::io::Cursor::new(identity.root.as_bytes());
            for cert in rustls_pemfile::certs(&mut root) {
                roots.add(cert.expect("parse workload root")).expect("add workload root");
            }
            let mut certs = Vec::new();
            for pem in [&identity.leaf, &identity.intermediate] {
                certs.extend(
                    rustls_pemfile::certs(&mut std::io::Cursor::new(pem.as_bytes()))
                        .map(|cert| cert.expect("parse remote certificate")),
                );
            }
            let key =
                rustls_pemfile::private_key(&mut std::io::Cursor::new(identity.key.as_bytes()))
                    .expect("parse remote key PEM")
                    .expect("remote key exists");
            let verifier = Arc::new(V6SpiffeVerifier::new(roots));
            let config = Arc::new(
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(verifier)
                    .with_client_auth_cert(certs, key)
                    .expect("remote client identity"),
            );
            let mut tcp = TcpStream::connect_timeout(&target.into(), Duration::from_secs(10))
                .expect("route-disjoint remote client reaches production leg C");
            tcp.set_read_timeout(Some(Duration::from_secs(10))).expect("remote read bound");
            let name = rustls::pki_types::ServerName::try_from("peer.overdrive.local")
                .expect("valid production SNI");
            let mut tls = rustls::ClientConnection::new(config, name).expect("TLS client");
            while tls.is_handshaking() {
                tls.complete_io(&mut tcp).expect("remote mTLS handshake");
            }
            let mut stream = rustls::Stream::new(&mut tls, &mut tcp);
            stream
                .write_all(overdrive_testing::guest_app::V6_REMOTE_REQUEST)
                .expect("remote inbound request");
            let mut response = vec![0u8; overdrive_testing::guest_app::V6_REMOTE_RESPONSE.len()];
            stream.read_exact(&mut response).expect("remote inbound response");
            assert_eq!(response, overdrive_testing::guest_app::V6_REMOTE_RESPONSE);
        })
        .join()
        .expect("remote mTLS fixture thread");
    }
}

#[derive(Debug)]
struct V6SpiffeVerifier {
    roots: rustls::RootCertStore,
    provider: Arc<CryptoProvider>,
}

impl V6SpiffeVerifier {
    fn new(roots: rustls::RootCertStore) -> Self {
        Self { roots, provider: Arc::new(rustls::crypto::ring::default_provider()) }
    }
}

impl ServerCertVerifier for V6SpiffeVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let parsed = ParsedCertificate::try_from(end_entity)?;
        verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        let (_, cert) = x509_parser::certificate::X509Certificate::from_der(end_entity.as_ref())
            .map_err(|_| RustlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        let san = cert
            .subject_alternative_name()
            .map_err(|_| RustlsError::InvalidCertificate(CertificateError::BadEncoding))?
            .ok_or(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))?;
        let uris = san
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [uri] = uris.as_slice() else {
            return Err(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        };
        SpiffeId::new(uri).map_err(|_| {
            RustlsError::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
        })?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub(super) struct EphemeralRangeGuard {
    original: String,
    low: u16,
    high: u16,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PopulationPlan {
    pub(super) count: u16,
    capacity: u16,
}

impl EphemeralRangeGuard {
    pub(super) fn narrow() -> Self {
        let path = "/proc/sys/net/ipv4/ip_local_port_range";
        let original = std::fs::read_to_string(path).expect("read original ephemeral range");
        let values = original
            .split_whitespace()
            .map(|value| value.parse::<u16>().expect("decimal ephemeral boundary"))
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        let high = values[1];
        let low = high.checked_sub(63).expect("the actual range holds 64 ports");
        assert!(low >= values[0], "64-port window lies inside the actual range");
        let occupied = socket_rows();
        assert!(
            occupied.iter().all(|row| !(low..=high).contains(&row.local.port())),
            "selected ephemeral window starts conflict-free: {occupied:?}"
        );
        std::fs::write(path, format!("{low} {high}\n")).expect("narrow disposable guest range");
        assert_eq!(
            std::fs::read_to_string(path).expect("read narrowed range").trim(),
            format!("{low}\t{high}")
        );
        Self { original, low, high }
    }

    fn free_capacity(&self) -> u16 {
        let occupied = occupied_loopback_ports(self.low, self.high);
        64_u16
            .checked_sub(u16::try_from(occupied.len()).expect("occupied count fits u16"))
            .expect("occupied ports do not exceed the selected range")
    }

    fn population_for_capacity(capacity: u16) -> PopulationPlan {
        let count = capacity / 2 + 1;
        assert!((2..=65).contains(&count), "bounded production population: C={capacity} N={count}");
        assert!(count < capacity, "production D sockets fit the measured capacity");
        assert!(2 * count > capacity, "the two-socket unset cohort cannot fit");
        PopulationPlan { count, capacity }
    }

    pub(super) fn run_unset_control(&self) -> PopulationPlan {
        let capacity = self.free_capacity();
        let plan = Self::population_for_capacity(capacity);
        let mut reservations = Vec::with_capacity(usize::from(plan.count));
        let mut reservation_ports = std::collections::BTreeSet::new();
        for _ in 0..plan.count {
            let socket = bind_loopback_without_no_port().expect("bind unset-control D reservation");
            let local = socket_local_v4(socket.as_raw_fd()).expect("read D reservation tuple");
            assert!((self.low..=self.high).contains(&local.port()));
            assert!(reservation_ports.insert(local.port()), "D reservations use distinct ports");
            reservations.push(socket);
        }
        assert_eq!(reservations.len(), usize::from(plan.count));

        let mut listeners = Vec::with_capacity(usize::from(plan.count));
        for index in 0..plan.count {
            listeners.push(
                TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), 20_000 + index))
                    .expect("bind distinct unset-control destination"),
            );
        }
        let expected_successes = capacity - plan.count;
        let mut clients = Vec::with_capacity(usize::from(expected_successes));
        let mut accepted = Vec::with_capacity(usize::from(expected_successes));
        let mut failure = None;
        for (index, listener) in listeners.iter().enumerate() {
            let destination = listener.local_addr().expect("control destination");
            match connect_bound_without_no_port(destination) {
                Ok(client) => {
                    assert!(failure.is_none(), "no success after exhaustion");
                    let (server, _) = listener.accept().expect("accept unset control");
                    clients.push(client);
                    accepted.push(server);
                }
                Err(error) => {
                    failure = Some((index, error));
                    break;
                }
            }
        }
        let (failed_at, error) = failure.expect("the first leg-B socket beyond C-N exhausts");
        assert_eq!(failed_at, clients.len());
        assert_eq!(clients.len(), usize::from(expected_successes));
        assert!(
            matches!(error.raw_os_error(), Some(libc::EADDRINUSE | libc::EADDRNOTAVAIL)),
            "unset exhaustion errno: {error}"
        );
        let source_ports = clients
            .iter()
            .map(|stream| stream.local_addr().expect("unset source").port())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(source_ports.len(), clients.len());
        assert!(source_ports.iter().all(|port| (self.low..=self.high).contains(port)));
        let SocketAddr::V4(failed_destination) =
            listeners[failed_at].local_addr().expect("failed destination")
        else {
            unreachable!("the unset control binds IPv4")
        };
        assert_no_live_tcp_rows(
            &std::collections::BTreeSet::from([failed_destination]),
            "connection N leaves no live socket row",
        );
        let control_destinations = listeners
            .iter()
            .map(|listener| match listener.local_addr().expect("control destination") {
                SocketAddr::V4(destination) => destination,
                SocketAddr::V6(_) => unreachable!("the unset control binds IPv4"),
            })
            .collect::<std::collections::BTreeSet<_>>();
        for stream in clients.iter().chain(&accepted) {
            reset_on_drop(stream);
        }
        drop((clients, accepted, listeners, reservations));
        assert_no_live_tcp_rows(
            &control_destinations,
            "the closed unset-control cohort leaves no live socket row",
        );
        assert_loopback_ports_absent(
            self.low,
            self.high,
            &reservation_ports,
            "the closed D-reservation cohort leaves no bound port",
        );
        assert_eq!(
            self.free_capacity(),
            capacity,
            "both reset-closed control socket classes restore the measured free capacity"
        );
        plan
    }

    pub(super) fn finish(mut self) {
        let original = std::mem::take(&mut self.original);
        std::fs::write("/proc/sys/net/ipv4/ip_local_port_range", &original)
            .expect("restore original ephemeral range");
        assert_eq!(
            std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
                .expect("read restored ephemeral range"),
            original
        );
    }
}

fn occupied_loopback_ports(low: u16, high: u16) -> std::collections::BTreeSet<u16> {
    std::fs::read_to_string("/proc/net/tcp")
        .expect("read complete TCP socket table")
        .lines()
        .skip(1)
        .filter_map(|line| {
            let local = line.split_whitespace().nth(1)?;
            let (address, port) = local.split_once(':')?;
            (address == "0100007F").then(|| u16::from_str_radix(port, 16).ok()).flatten()
        })
        .filter(|port| (low..=high).contains(port))
        .collect()
}

fn assert_loopback_ports_absent(
    low: u16,
    high: u16,
    ports: &std::collections::BTreeSet<u16>,
    context: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let occupied = occupied_loopback_ports(low, high)
            .intersection(ports)
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if occupied.is_empty() {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{context}: {occupied:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl Drop for EphemeralRangeGuard {
    fn drop(&mut self) {
        let _ = std::fs::write("/proc/sys/net/ipv4/ip_local_port_range", &self.original);
    }
}

fn socklen_of<T>(value: &T) -> libc::socklen_t {
    libc::socklen_t::try_from(std::mem::size_of_val(value)).expect("socket structure size fits")
}

fn reset_on_drop(stream: &TcpStream) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    assert_eq!(
        unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                std::ptr::from_ref(&linger).cast(),
                socklen_of(&linger),
            )
        },
        0,
        "arm reset-close for the disposable unset cohort: {}",
        std::io::Error::last_os_error()
    );
}

fn bind_loopback_without_no_port() -> std::io::Result<OwnedFd> {
    // SAFETY: the returned OwnedFd takes sole ownership after a successful
    // IPv4 socket creation; every error path closes through OwnedFd::drop.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let option = socket_no_port(socket.as_raw_fd())?;
    assert_eq!(option, 0, "unset control reads back option 0");
    let local = libc::sockaddr_in {
        sin_family: u16::try_from(libc::AF_INET).expect("AF_INET fits sa_family_t"),
        sin_port: 0,
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(Ipv4Addr::LOCALHOST.octets()) },
        sin_zero: [0; 8],
    };
    if unsafe {
        libc::bind(socket.as_raw_fd(), std::ptr::from_ref(&local).cast(), socklen_of(&local))
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(socket)
}

fn socket_local_v4(fd: RawFd) -> std::io::Result<SocketAddrV4> {
    let mut local = libc::sockaddr_in {
        sin_family: 0,
        sin_port: 0,
        sin_addr: libc::in_addr { s_addr: 0 },
        sin_zero: [0; 8],
    };
    let mut len = socklen_of(&local);
    if unsafe {
        libc::getsockname(fd, std::ptr::from_mut(&mut local).cast(), std::ptr::from_mut(&mut len))
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SocketAddrV4::new(
        Ipv4Addr::from(local.sin_addr.s_addr.to_ne_bytes()),
        u16::from_be(local.sin_port),
    ))
}

fn connect_bound_without_no_port(destination: std::net::SocketAddr) -> std::io::Result<TcpStream> {
    let socket = bind_loopback_without_no_port()?;
    let std::net::SocketAddr::V4(destination) = destination else { unreachable!() };
    let remote = libc::sockaddr_in {
        sin_family: u16::try_from(libc::AF_INET).expect("AF_INET fits sa_family_t"),
        sin_port: destination.port().to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(destination.ip().octets()) },
        sin_zero: [0; 8],
    };
    if unsafe {
        libc::connect(socket.as_raw_fd(), std::ptr::from_ref(&remote).cast(), socklen_of(&remote))
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { TcpStream::from_raw_fd(socket.into_raw_fd()) })
}

fn socket_no_port(fd: RawFd) -> std::io::Result<i32> {
    let mut value: libc::c_int = -1;
    let mut len = socklen_of(&value);
    if unsafe {
        libc::getsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_BIND_ADDRESS_NO_PORT,
            std::ptr::from_mut(&mut value).cast(),
            std::ptr::from_mut(&mut len),
        )
    } == 0
    {
        Ok(value)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[derive(Debug, Clone)]
pub(super) struct SocketRow {
    pub(super) local: SocketAddrV4,
    pub(super) peer: SocketAddrV4,
    pub(super) inode: u64,
    pub(super) record: String,
}

pub(super) fn socket_rows() -> Vec<SocketRow> {
    let out = Command::new("ss")
        .args(["-H", "-4", "-t", "-n", "-e", "-i", "state", "established"])
        .output()
        .expect("run ss for V-6 socket evidence");
    assert!(out.status.success(), "ss: {}", String::from_utf8_lossy(&out.stderr));
    let output = String::from_utf8_lossy(&out.stdout);
    let mut rows: Vec<SocketRow> = Vec::new();
    for line in output.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let header = fields
            .get(2)
            .and_then(|local| local.parse().ok())
            .zip(fields.get(3).and_then(|peer| peer.parse().ok()))
            .zip(
                fields
                    .iter()
                    .find_map(|field| field.strip_prefix("ino:").and_then(|v| v.parse().ok())),
            );
        if let Some(((local, peer), inode)) = header {
            rows.push(SocketRow { local, peer, inode, record: line.to_owned() });
        } else if let Some(row) = rows.last_mut() {
            row.record.push(' ');
            row.record.push_str(line.trim());
        }
    }
    rows
}

#[derive(Debug)]
struct TcpTableRow {
    local: SocketAddrV4,
    peer: SocketAddrV4,
    state: u8,
}

fn tcp_table_rows() -> Vec<TcpTableRow> {
    std::fs::read_to_string("/proc/net/tcp")
        .expect("read complete TCP socket table")
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            Some(TcpTableRow {
                local: proc_tcp_endpoint(fields.get(1)?)?,
                peer: proc_tcp_endpoint(fields.get(2)?)?,
                state: u8::from_str_radix(fields.get(3)?, 16).ok()?,
            })
        })
        .collect()
}

fn proc_tcp_endpoint(field: &str) -> Option<SocketAddrV4> {
    let (address, port) = field.split_once(':')?;
    let address = u32::from_str_radix(address, 16).ok()?;
    let port = u16::from_str_radix(port, 16).ok()?;
    Some(SocketAddrV4::new(Ipv4Addr::from(address.to_le_bytes()), port))
}

fn assert_no_live_tcp_rows(endpoints: &std::collections::BTreeSet<SocketAddrV4>, context: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let rows = tcp_table_rows()
            .into_iter()
            .filter(|row| {
                !matches!(row.state, 0x06 | 0x07 | 0x0a)
                    && (endpoints.contains(&row.local) || endpoints.contains(&row.peer))
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{context}: {rows:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn inode_fds(inode: u64) -> Vec<RawFd> {
    let needle = format!("socket:[{inode}]");
    std::fs::read_dir("/proc/self/fd")
        .expect("read self fd table")
        .flatten()
        .filter_map(|entry| {
            (std::fs::read_link(entry.path()).ok()?.to_string_lossy() == needle)
                .then(|| entry.file_name().to_string_lossy().parse().ok())
                .flatten()
        })
        .collect()
}

fn population_backends(
    peer_addr: Ipv4Addr,
    population: u16,
) -> std::collections::BTreeSet<SocketAddrV4> {
    (0..population)
        .map(|index| SocketAddrV4::new(peer_addr, V6_POPULATION_BASE_PORT + index))
        .collect()
}

pub(super) fn assert_population_option_set(
    range: &EphemeralRangeGuard,
    peer_addr: Ipv4Addr,
    plan: PopulationPlan,
) {
    let population = plan.count;
    let expected = population_backends(peer_addr, population);
    let rows =
        socket_rows().into_iter().filter(|row| expected.contains(&row.peer)).collect::<Vec<_>>();
    assert_eq!(rows.len(), usize::from(population), "all production population tuples held");
    let mut sources = std::collections::BTreeSet::new();
    let mut destinations = std::collections::BTreeSet::new();
    for row in &rows {
        destinations.insert(row.peer);
        sources.insert(row.local.port());
        assert!(
            (range.low..=range.high).contains(&row.local.port()),
            "production source port lies inside the narrowed range: {row:?}"
        );
        let fds = inode_fds(row.inode);
        assert_eq!(fds.len(), 1, "one live fd owns production leg-B inode {}", row.inode);
        assert_eq!(socket_no_port(fds[0]).expect("read production no-port option"), 1);
    }
    assert_eq!(destinations, expected, "the exact registered backend set is held");
    assert!(sources.len() < usize::from(population), "set cohort shares source ports");
    assert!(
        usize::from(population) + sources.len() <= usize::from(plan.capacity),
        "production D plus shared leg-B sources fit measured C: N={} S={} C={}",
        population,
        sources.len(),
        plan.capacity
    );
    assert!(2 * population > plan.capacity, "the same unset two-socket cohort cannot fit");
}

pub(super) fn assert_population_absent(peer_addr: Ipv4Addr, population: u16) {
    let expected = population_backends(peer_addr, population);
    assert_no_live_tcp_rows(&expected, "the closed production population leaves no live row");
}

pub(super) struct UplinkGuard {
    pub(super) iface: String,
    namespace: String,
}

impl UplinkGuard {
    pub(super) fn create() -> Self {
        let iface = "ovdv6up0".to_owned();
        let peer = "ovdv6up1";
        let namespace = "ovd-v6-sink".to_owned();
        let _ = ip(&["link", "del", &iface]);
        let _ = ip(&["netns", "del", &namespace]);
        ip_ok(&["netns", "add", &namespace]);
        ip_ok(&["link", "add", &iface, "type", "veth", "peer", "name", peer]);
        ip_ok(&["link", "set", peer, "netns", &namespace]);
        ip_ok(&["addr", "add", "198.18.0.1/30", "dev", &iface]);
        ip_ok(&["link", "set", &iface, "up"]);
        ip_ok(&["netns", "exec", &namespace, "ip", "addr", "add", "198.18.0.2/30", "dev", peer]);
        ip_ok(&["netns", "exec", &namespace, "ip", "link", "set", peer, "up"]);
        ip_ok(&["route", "add", "default", "via", "198.18.0.2", "dev", &iface, "metric", "42662"]);
        let route = ip(&["-4", "route", "show", "default"]).expect("read default route");
        assert!(route.contains("default via 198.18.0.2 dev ovdv6up0 metric 42662"));
        let link = ip(&["-details", "link", "show", "dev", &iface]).expect("read uplink");
        assert!(link.contains("UP"));
        Self { iface, namespace }
    }

    pub(super) fn positive_control() {
        let socket = std::net::UdpSocket::bind((Ipv4Addr::new(198, 18, 0, 1), 0))
            .expect("bind uplink positive control");
        socket
            .send_to(b"V6-UPLINK-POSITIVE", (Ipv4Addr::new(198, 18, 0, 2), 9))
            .expect("send uplink positive control");
    }

    pub(super) fn finish(mut self) {
        let iface = std::mem::take(&mut self.iface);
        let namespace = std::mem::take(&mut self.namespace);
        ip_ok(&["route", "del", "default", "via", "198.18.0.2", "dev", &iface, "metric", "42662"]);
        ip_ok(&["link", "del", &iface]);
        ip_ok(&["netns", "del", &namespace]);
        assert!(ip(&["link", "show", "dev", &iface]).is_err());
        assert!(!ip(&["-4", "route", "show", "default"]).unwrap_or_default().contains(&iface));
    }
}

impl Drop for UplinkGuard {
    fn drop(&mut self) {
        let _ = ip(&[
            "route",
            "del",
            "default",
            "via",
            "198.18.0.2",
            "dev",
            &self.iface,
            "metric",
            "42662",
        ]);
        let _ = ip(&["link", "del", &self.iface]);
        let _ = ip(&["netns", "del", &self.namespace]);
    }
}

fn ip(args: &[&str]) -> Result<String, String> {
    let output = Command::new("ip").args(args).output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr)))
    }
}

fn ip_ok(args: &[&str]) {
    ip(args).unwrap_or_else(|error| panic!("{error}"));
}

#[derive(Debug)]
pub(super) struct PacketEvidence {
    packets: Vec<CapturedPacket>,
    drops: u32,
}

#[derive(Debug)]
struct CapturedPacket {
    bytes: Vec<u8>,
    wire_len: usize,
    packet_type: u8,
    truncated: bool,
    control_truncated: bool,
}

pub(super) struct PacketCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<PacketEvidence>>,
}

impl PacketCapture {
    pub(super) fn start(iface: &str) -> Self {
        let fd = open_packet_socket(iface);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || capture_packets(fd, &thread_stop));
        Self { stop, thread: Some(thread) }
    }

    pub(super) fn finish(mut self) -> PacketEvidence {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().expect("capture thread").join().expect("capture thread joins")
    }
}

impl Drop for PacketCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn open_packet_socket(iface: &str) -> RawFd {
    let iface = std::ffi::CString::new(iface).expect("interface has no NUL");
    // SAFETY: libc retains no pointer.
    let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
    assert_ne!(ifindex, 0, "capture interface exists");
    // SAFETY: create one packet socket, configured and closed by this helper.
    let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0, "open AF_PACKET: {}", std::io::Error::last_os_error());
    let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    addr.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits sa_family_t");
    addr.sll_protocol = 0x0003_u16.to_be();
    addr.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits i32");
    assert_eq!(
        unsafe { libc::bind(fd, std::ptr::from_ref(&addr).cast(), socklen_of(&addr),) },
        0,
        "bind AF_PACKET: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: fcntl operates on the live capture fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0);
    fd
}

fn capture_packets(fd: RawFd, stop: &AtomicBool) -> PacketEvidence {
    let mut packets = Vec::new();
    let mut buffer = vec![0u8; 65_535];
    while !stop.load(Ordering::SeqCst) {
        match receive_packet(fd, &mut buffer) {
            Ok(Some(packet)) => packets.push(packet),
            Ok(None) => std::thread::sleep(Duration::from_micros(200)),
            Err(error) => panic!("AF_PACKET receive: {error}"),
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    while let Some(packet) = receive_packet(fd, &mut buffer).expect("final capture drain") {
        packets.push(packet);
    }
    let mut statistics = [0u32; 2];
    let mut length = socklen_of(&statistics);
    assert_eq!(
        unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_PACKET,
                6,
                statistics.as_mut_ptr().cast(),
                std::ptr::from_mut(&mut length),
            )
        },
        0,
        "read PACKET_STATISTICS"
    );
    unsafe { libc::close(fd) };
    PacketEvidence { packets, drops: statistics[1] }
}

fn receive_packet(fd: RawFd, buffer: &mut [u8]) -> std::io::Result<Option<CapturedPacket>> {
    let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    let mut iov = libc::iovec { iov_base: buffer.as_mut_ptr().cast(), iov_len: buffer.len() };
    let mut control: [libc::cmsghdr; 4] = unsafe { std::mem::zeroed() };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_name = std::ptr::from_mut(&mut addr).cast();
    message.msg_namelen = socklen_of(&addr);
    message.msg_iov = std::ptr::from_mut(&mut iov);
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&control);
    let n = unsafe { libc::recvmsg(fd, &raw mut message, libc::MSG_TRUNC) };
    if n < 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::WouldBlock { Ok(None) } else { Err(error) };
    }
    let wire_len = usize::try_from(n).expect("packet length nonnegative");
    let copied = wire_len.min(buffer.len());
    Ok(Some(CapturedPacket {
        bytes: buffer[..copied].to_vec(),
        wire_len,
        packet_type: addr.sll_pkttype,
        truncated: wire_len > copied || message.msg_flags & libc::MSG_TRUNC != 0,
        control_truncated: message.msg_flags & libc::MSG_CTRUNC != 0,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TcpTuple {
    source: SocketAddrV4,
    destination: SocketAddrV4,
}

type TcpPayload<'a> = (TcpTuple, u32, &'a [u8]);

fn tcp_payload(packet: &CapturedPacket) -> Result<Option<TcpPayload<'_>>, String> {
    if packet.truncated || packet.control_truncated || packet.bytes.len() != packet.wire_len {
        return Err("truncated packet capture".to_owned());
    }
    let bytes = &packet.bytes;
    if bytes.len() < 40 || bytes[0] >> 4 != 4 {
        return Ok(None);
    }
    let ip_len = usize::from(bytes[0] & 0x0f) * 4;
    if ip_len < 20 || bytes.len() < ip_len + 20 || bytes[9] != 6 {
        return Ok(None);
    }
    let total = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if total > bytes.len() || total < ip_len + 20 {
        return Err("ambiguous IPv4 total length".to_owned());
    }
    let tcp = &bytes[ip_len..total];
    let tcp_len = usize::from(tcp[12] >> 4) * 4;
    if tcp_len < 20 || tcp_len > tcp.len() {
        return Err("ambiguous TCP header length".to_owned());
    }
    let source = SocketAddrV4::new(
        Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]),
        u16::from_be_bytes([tcp[0], tcp[1]]),
    );
    let destination = SocketAddrV4::new(
        Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]),
        u16::from_be_bytes([tcp[2], tcp[3]]),
    );
    let sequence = u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]);
    Ok(Some((TcpTuple { source, destination }, sequence, &tcp[tcp_len..])))
}

impl PacketEvidence {
    fn assert_lossless(&self) {
        assert_eq!(self.drops, 0, "AF_PACKET reports no drops");
        for packet in &self.packets {
            assert!(!packet.truncated && !packet.control_truncated);
            assert_eq!(packet.bytes.len(), packet.wire_len);
        }
    }

    pub(super) fn assert_positive_control_and_no_tuple(&self, backend: SocketAddrV4) {
        self.assert_lossless();
        assert!(
            self.packets.iter().any(|packet| packet
                .bytes
                .windows(b"V6-UPLINK-POSITIVE".len())
                .any(|window| window == b"V6-UPLINK-POSITIVE")),
            "uplink capture observed its positive control"
        );
        let matching = self
            .packets
            .iter()
            .filter_map(|packet| tcp_payload(packet).transpose())
            .collect::<Result<Vec<_>, _>>()
            .expect("uplink packet parsing is unambiguous")
            .into_iter()
            .filter(|(tuple, _, _)| tuple.destination == backend || tuple.source == backend)
            .count();
        assert_eq!(matching, 0, "same-node backend tuple emitted no uplink frame");
    }

    pub(super) fn assert_tls13_no_cleartext(&self, backend: SocketAddrV4) {
        self.assert_lossless();
        let backend_rows =
            socket_rows().into_iter().filter(|row| row.peer == backend).collect::<Vec<_>>();
        let candidates = backend_rows
            .iter()
            .filter(|row| {
                row.record.contains("tcp-ulp-tls")
                    && (row.record.contains("version:1.3") || row.record.contains("version: 1.3"))
                    && (row.record.contains("txconf:sw") || row.record.contains("txconf: sw"))
                    && (row.record.contains("rxconf:sw") || row.record.contains("rxconf: sw"))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            candidates.len(),
            1,
            "one exact bidirectional TLS 1.3 leg-F socket; backend rows: {backend_rows:#?}"
        );
        assert_eq!(inode_fds(candidates[0].inode).len(), 1, "one fd owner of the kTLS inode");
        let tuple = TcpTuple { source: candidates[0].local, destination: candidates[0].peer };
        let mut directions = std::collections::BTreeSet::new();
        let mut streams: std::collections::BTreeMap<
            TcpTuple,
            std::collections::BTreeMap<u32, Vec<u8>>,
        > = std::collections::BTreeMap::new();
        for packet in &self.packets {
            if let Some((observed, sequence, payload)) =
                tcp_payload(packet).expect("loopback packet parsing is unambiguous")
                && (observed == tuple
                    || observed
                        == TcpTuple { source: tuple.destination, destination: tuple.source })
                && !payload.is_empty()
            {
                streams
                    .entry(observed)
                    .or_default()
                    .entry(sequence)
                    .or_insert_with(|| payload.to_vec());
            }
        }
        for (observed, chunks) in streams {
            let bytes = chunks.into_values().flatten().collect::<Vec<_>>();
            assert!(
                !bytes
                    .windows(overdrive_testing::guest_app::V6_VIP_REQUEST.len())
                    .any(|window| window == overdrive_testing::guest_app::V6_VIP_REQUEST)
            );
            assert!(
                !bytes
                    .windows(overdrive_testing::guest_app::V6_VIP_RESPONSE.len())
                    .any(|window| window == overdrive_testing::guest_app::V6_VIP_RESPONSE)
            );
            assert!(
                bytes.windows(5).any(|window| {
                    window[0] == 0x17
                        && window[1] == 0x03
                        && window[2] == 0x03
                        && usize::from(u16::from_be_bytes([window[3], window[4]])) > 0
                }),
                "TLS application-data record appears on {observed:?}"
            );
            directions.insert(observed);
        }
        assert_eq!(directions.len(), 2, "TLS application data appears in both directions");
    }
}

/// A row's allocation id.
pub(super) fn alloc_of(row: &AllocStatusRowBody) -> AllocationId {
    AllocationId::new(&row.alloc_id).expect("a valid allocation id")
}

/// The pid and NUL-joined argv of the live `cloud-hypervisor` process of
/// `alloc` (matched by its run directory in the argv), or `None`.
pub(super) fn hypervisor_of(alloc: &AllocationId) -> Option<(u32, String)> {
    let needle =
        VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).path().to_string_lossy().into_owned();
    for entry in std::fs::read_dir("/proc").expect("read /proc").flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or(&[]);
        if !String::from_utf8_lossy(argv0).ends_with("cloud-hypervisor") {
            continue;
        }
        let argv = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if argv.contains(&needle) && process_alive(pid) {
            return Some((pid, argv));
        }
    }
    None
}

/// The CID of a VMM's one `--vsock cid=<cid>,backend=vhost-kernel` device.
pub(super) fn vsock_cid(argv: &str) -> u32 {
    let at = argv.find("cid=").unwrap_or_else(|| panic!("no --vsock cid= in {argv}"));
    argv[at + 4..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("a decimal CID in {argv}"))
}

/// Whether `pid` is a live (not zombie) process.
pub(super) fn process_alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(") ").map(|(_, rest)| !rest.starts_with('Z')))
        .unwrap_or(false)
}

/// Waits up to `bound` for `pid` to end.
pub(super) async fn wait_process_gone(pid: u32, bound: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + bound;
    while process_alive(pid) {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    true
}

/// The allocation's guest console so far.
pub(super) fn console_of(alloc: &AllocationId) -> String {
    let path = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).console_log();
    let live = std::fs::read(path).unwrap_or_default();
    let bytes = if live.is_empty() {
        retained_consoles()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&alloc.to_string())
            .cloned()
            .unwrap_or_default()
    } else {
        live
    };
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Waits up to `bound` for the allocation's console to satisfy `done`.
pub(super) async fn wait_console(
    alloc: &AllocationId,
    bound: Duration,
    done: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        let console = console_of(alloc);
        if done(&console) {
            return Ok(console);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(console);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Formats an address for a guest application argument.
pub(super) fn arg(addr: SocketAddrV4) -> String {
    addr.to_string()
}
