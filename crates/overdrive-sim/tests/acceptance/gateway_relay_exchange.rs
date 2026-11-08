//! Seeded kernel-relay exchange runs over `SimGatewayRelay` (S-PIG-102 and
//! S-PIG-103).
//!
//! Both tests drive the production `UnboundGatewayBuilder` -> `GatewayBuilder`
//! composition over a real redb Route store and the Sim relay, connect dataplane,
//! client mTLS and identity ports. A real client socket connects to the gateway's
//! own `127/8` address on port 443; the scripted TLS peers behind it are the Sim
//! relay scripts, and the oracle is what the gateway wrote to each leg (the Sim
//! per-leg transcript), the injected clock, the shutdown report and the dataplane
//! live counts. There is no `Invariant` variant for these runs: they are seeded
//! acceptance tests, and the seed of every case is printed before it runs.
//!
//! # Bind addresses (R-6)
//!
//! Every case composes its own gateway on `127.K.hi.lo` (`K` = 122 for S-PIG-102,
//! 123 for S-PIG-103; `hi = seed / 250 % 200`, `lo = seed % 250 + 1`), so no two
//! cases of one test, and no two tests, share an address. The Service frontend
//! is `127.K.254.lo`.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    reason = "CONTRACT_SHAPE metadata, seeded scenario bodies, the printed seed and fixture preconditions"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::num::NonZeroU16;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::gateway_identity::{GatewayIdentityDesired, GatewayIdentityEpoch};
use overdrive_core::id::{AllocationId, ContentHash, NodeId, ServiceVip, WorkloadId};
use overdrive_core::public_ingress::{
    GatewayListenerStatus, PublicCertifiedKeyId, PublicHostname, PublicRouteInput, Route,
    ServiceListenerReferenceInput,
};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{Ca, SvidRequest};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayHandle, GatewayRouteCommands, GatewayRuntimeDependencies, GatewayShutdownReport,
    GatewayStatusReads, UnboundGatewayBuilder,
};
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectDataplane, GatewayIdentityActionTarget,
};
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::gateway::{
    LegRole, LegScript, LegTranscript, PublicLegScript, RelayCall, SimGatewayClientMtls,
    SimGatewayConnectDataplane, SimGatewayIdentityActionTarget, SimGatewayIdentityLifecycleControl,
    SimGatewayRelay,
};
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::LocalIntentStore;
use rand::rngs::StdRng;
use rand::{Rng as _, SeedableRng};
use tempfile::TempDir;

const HOSTNAME: &str = "api.example.com";
const SERVICE_PORT: u16 = 8080;
const GRACE: Duration = Duration::from_secs(5);
const WAIT: Duration = Duration::from_secs(20);
const CASES: u64 = 48;

// ---------------------------------------------------------------------------
// One production composition over the Sim relay
// ---------------------------------------------------------------------------

struct Rig {
    _dir: TempDir,
    clock: Arc<SimClock>,
    relay: Arc<SimGatewayRelay>,
    connect: Arc<SimGatewayConnectDataplane>,
    _routes: GatewayRouteCommands,
    _status: GatewayStatusReads,
    handle: Option<GatewayHandle>,
    bind: SocketAddrV4,
    _frontend_listener: std::net::TcpListener,
}

impl Rig {
    async fn start(kind: u8, seed: u64) -> Self {
        let lo = (seed % 250) as u8 + 1;
        let hi = (seed / 250 % 200) as u8;
        let bind = Ipv4Addr::new(127, kind, hi, lo);
        let vip = Ipv4Addr::new(127, kind, 254, lo);
        let dir = tempfile::tempdir().expect("isolated gateway root");
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
        let mut params =
            rcgen::CertificateParams::new(vec![HOSTNAME.to_owned()]).expect("certificate params");
        params.is_ca = rcgen::IsCa::NoCa;
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2100, 1, 1);
        let cert = params.self_signed(&key).expect("test certificate");
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        std::fs::write(&chain_path, cert.pem()).expect("chain");
        std::fs::write(&key_path, key.serialize_pem()).expect("key");
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .expect("private-key mode");

        let clock = Arc::new(SimClock::new());
        let intent =
            Arc::new(LocalIntentStore::open(dir.path().join("intent.redb")).expect("intent"));
        let node = NodeId::new("gateway-node").expect("node ID");
        let observations = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
        let vip_view = persist_api_service(&intent, vip).await;
        let listener = std::net::TcpListener::bind((vip, SERVICE_PORT)).expect("frontend");
        listener.set_nonblocking(true).expect("nonblocking frontend");
        let frontend = ServiceFrontend::new(
            ServiceVip::new(vip.into()).expect("VIP"),
            NonZeroU16::new(SERVICE_PORT).expect("port"),
            Proto::Tcp,
        )
        .expect("frontend");
        let workload = WorkloadId::new("api").expect("workload");
        let backend_identity = SpiffeId::for_allocation(
            &workload,
            &AllocationId::new("a1b2c3d4").expect("allocation"),
        );
        let dataplane = Arc::new(SimDataplane::new());
        dataplane
            .update_service(
                frontend,
                vec![Backend {
                    alloc: backend_identity.clone(),
                    addr: std::net::SocketAddr::from((vip, SERVICE_PORT)),
                    weight: 1,
                    healthy: true,
                }],
            )
            .await
            .expect("program the Service");
        let connect = Arc::new(SimGatewayConnectDataplane::new(dataplane));
        let relay = Arc::new(SimGatewayRelay::new(clock.clone()));

        let identity = Arc::new(SimGatewayIdentityActionTarget::new(GatewayIdentityDesired {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: Some(SpiffeId::for_gateway(&node)),
        }));
        let now = clock.unix_now();
        let material = SimCa::new(Arc::new(SimEntropy::new(seed)))
            .issue_svid(&SvidRequest::new(
                SpiffeId::for_gateway(&node),
                UnixInstant::from_unix_duration(now),
                UnixInstant::from_unix_duration(now + Duration::from_secs(86_400)),
            ))
            .expect("gateway SVID");
        identity.hold_audited(GatewayIdentityEpoch::first(), material);

        let kek = Arc::new(SimKek::new().with(
            &KekId::new("overdrive-public-certified-key").expect("custody KEK ID"),
            [0x5a; KEK_LEN],
        ));
        let config = GatewayConfig::new(
            bind,
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                chain_path,
                key_path,
            ),
        );
        let intent_port: Arc<dyn IntentStore> = intent;
        let observation_port: Arc<dyn ObservationStore> = observations;
        let clock_port: Arc<dyn Clock> = clock.clone();
        let (builder, seal) = UnboundGatewayBuilder::new(
            node.clone(),
            config,
            intent_port,
            observation_port,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(kek)),
            clock_port.clone(),
        )
        .await
        .expect("gateway builder")
        .into_builder();
        let client = SimGatewayClientMtls::new(seal, relay.clone(), clock_port.clone());
        client.set_peer(backend_identity);
        let lifecycle = SimGatewayIdentityLifecycleControl::new(
            identity,
            Arc::new(parking_lot::Mutex::new(EvaluationBroker::new())),
            Evaluation {
                reconciler: ReconcilerName::new("gateway-svid-lifecycle").expect("reconciler"),
                target: TargetResource::new("node/gateway-node").expect("target"),
            },
            clock_port,
        );
        let connect_port: Arc<dyn GatewayConnectDataplane> = connect.clone();
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            connect_port,
            Arc::new(client),
            Arc::new(lifecycle),
            relay.clone(),
        );
        let (handle, routes, status) = builder.start(dependencies).await.into_parts();
        routes.routes().apply(route()).await.expect("declare the Route");
        loop {
            let row = status.application_status().await.expect("status read");
            if let Some(row) = row
                && row.current.is_some()
                && row.unavailable.is_none()
                && matches!(row.listener, GatewayListenerStatus::Bound { .. })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            _dir: dir,
            clock,
            relay,
            connect,
            _routes: routes,
            _status: status,
            handle: Some(handle),
            bind: SocketAddrV4::new(bind, 443),
            _frontend_listener: listener,
        }
    }

    /// Queue one public connection and the origin legs, then connect a real
    /// client socket. Returns the index of the first leg the exchange mints.
    fn start_exchange(&self, public: LegScript, upstreams: Vec<LegScript>) -> (usize, TcpStream) {
        let first = self.relay.leg_transcripts().len();
        self.relay.script_public_connection(PublicLegScript {
            sni: PublicHostname::new(HOSTNAME).expect("SNI"),
            inbound: public,
        });
        for upstream in upstreams {
            self.relay.script_upstream_leg(upstream);
        }
        let client = TcpStream::connect(self.bind).expect("connect to the gateway listener");
        (first, client)
    }

    fn legs_since(&self, first: usize) -> Vec<LegTranscript> {
        self.relay.leg_transcripts().into_iter().filter(|leg| leg.id.0 >= first as u64).collect()
    }

    /// Run until the public leg exists and every leg of the exchange is closed,
    /// advancing the injected clock.
    async fn finish(&self, first: usize) -> Vec<LegTranscript> {
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            let legs = self.legs_since(first);
            if legs.iter().any(|leg| leg.role == LegRole::Public)
                && legs.iter().all(|leg| leg.closed)
            {
                return legs;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for the exchange to end with every leg closed: {legs:?}"
            );
            self.clock.tick(Duration::from_millis(100));
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
    }

    async fn shutdown(&mut self) -> GatewayShutdownReport {
        let handle = self.handle.take().expect("gateway handle");
        drive_clock(&self.clock, Duration::from_millis(250), handle.shutdown(GRACE)).await
    }

    /// The residue oracle: zero legs, pipes, intents and receipts.
    fn assert_no_residue(&self, what: &str) {
        assert_eq!(self.relay.live_leg_count(), 0, "{what}: live legs remain");
        assert_eq!(self.relay.live_pipe_count(), 0, "{what}: live pipes remain");
        assert_eq!(self.connect.live_intent_count().ok(), Some(0), "{what}: live intents remain");
        assert_eq!(
            self.connect.live_receipts().ok().map(|receipts| receipts.len()),
            Some(0),
            "{what}: live receipts remain"
        );
    }
}

async fn drive_clock<T>(clock: &SimClock, step: Duration, future: impl Future<Output = T>) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            () = tokio::time::sleep(Duration::from_millis(3)) => clock.tick(step),
        }
    }
}

async fn wait_until(what: &str, mut probe: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !probe() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn route() -> Route {
    Route::from_submit(PublicRouteInput {
        id: "public-api".to_owned(),
        hostname: HOSTNAME.to_owned(),
        path: "/".to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: SERVICE_PORT,
            protocol: "tcp".to_owned(),
        },
    })
    .expect("valid Route")
}

async fn persist_api_service(intent: &LocalIntentStore, vip: Ipv4Addr) -> Arc<SimServiceVipView> {
    let service = ServiceV1::from_submit(ServiceSpecInput {
        id: "api".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
        startup_probes: Vec::new(),
        readiness_probes: Vec::new(),
        liveness_probes: Vec::new(),
    })
    .expect("valid api Service input");
    let workload_id = service.id.clone();
    let service_intent = WorkloadIntent::Service(service);
    let digest = service_intent.spec_digest().expect("digest");
    let archived = service_intent.archive_for_store().expect("archive");
    intent
        .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
        .await
        .expect("persist api Service");
    intent
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("persist api Service kind");
    Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::from([(
        digest,
        ServiceVip::new(vip.into()).expect("Service VIP"),
    )])))
}

// ---------------------------------------------------------------------------
// Wire builders and head/body splitting
// ---------------------------------------------------------------------------

fn request_head(method: &str, target: &str, length: usize) -> Vec<u8> {
    format!("{method} {target} HTTP/1.1\r\nHost: {HOSTNAME}\r\nContent-Length: {length}\r\n\r\n")
        .into_bytes()
}

fn response_wire(status: u16, body: &[u8]) -> Vec<u8> {
    let mut wire =
        format!("HTTP/1.1 {status} Status\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    wire.extend_from_slice(body);
    wire
}

/// Octets whose values cycle so that a shifted, dropped or duplicated span is
/// visible; `salt` distinguishes request from response bodies.
fn pattern(len: usize, salt: u8) -> Vec<u8> {
    (0..len).map(|index| ((index % 251) as u8).wrapping_add(salt)).collect()
}

/// Split `wire` at the head/body boundary: the head including its blank line,
/// and the octets after it.
fn split_message(wire: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = wire.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
    Some(wire.split_at(end))
}

fn status_of(head: &[u8]) -> Option<u16> {
    std::str::from_utf8(head).ok()?.split_whitespace().nth(1)?.parse().ok()
}

/// Add the seeded control events (harmless ones) between `chunks`.
fn script_with_controls(
    mut script: LegScript,
    chunks: &[Vec<u8>],
    rng: &mut StdRng,
    harmless: bool,
) -> LegScript {
    for chunk in chunks {
        if harmless {
            script = match rng.gen_range(0..4_u8) {
                0 => script.key_update(false),
                1 => script.key_update(true),
                2 => script.new_session_ticket(),
                _ => script,
            };
        }
        script = script.octets(chunk.clone());
    }
    script
}

/// Cut `wire` into 1..=4 pieces at seeded offsets.
fn cut(wire: &[u8], rng: &mut StdRng) -> Vec<Vec<u8>> {
    let pieces = rng.gen_range(1..=4_usize).min(wire.len().max(1));
    let mut points: Vec<usize> = (1..pieces).map(|_| rng.gen_range(1..wire.len().max(2))).collect();
    points.sort_unstable();
    points.dedup();
    let mut out = Vec::new();
    let mut start = 0;
    for point in points.into_iter().chain(std::iter::once(wire.len())) {
        if point > start {
            out.push(wire[start..point].to_vec());
            start = point;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// S-PIG-102
// ---------------------------------------------------------------------------

/// How an exchange of the seeded schedule ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Harmless control records only: the exchange completes octet-exact.
    Completes,
    /// The origin ends the connection before any response octet.
    OriginClosesBeforeHead(OriginEnd),
    /// The origin delivers the head and part of the body, then fails fatally.
    OriginFailsMidBody(OriginEnd),
    /// The client fails fatally in the middle of its request body.
    ClientFailsMidBody(OriginEnd),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginEnd {
    CloseNotify,
    Alert,
    UnexpectedRecord,
    Reset,
}

fn end_with(script: LegScript, end: OriginEnd) -> LegScript {
    match end {
        OriginEnd::CloseNotify => script.close_notify(),
        OriginEnd::Alert => script.alert(40),
        OriginEnd::UnexpectedRecord => script.unexpected_record(0x15),
        OriginEnd::Reset => script.reset(),
    }
}

fn pick_end(rng: &mut StdRng) -> OriginEnd {
    match rng.gen_range(0..4_u8) {
        0 => OriginEnd::CloseNotify,
        1 => OriginEnd::Alert,
        2 => OriginEnd::UnexpectedRecord,
        _ => OriginEnd::Reset,
    }
}

fn pick_ending(rng: &mut StdRng) -> Ending {
    match rng.gen_range(0..4_u8) {
        0 => Ending::Completes,
        1 => Ending::OriginClosesBeforeHead(pick_end(rng)),
        2 => Ending::OriginFailsMidBody(pick_end(rng)),
        _ => Ending::ClientFailsMidBody(pick_end(rng)),
    }
}

/// S-PIG-102: under scripted peer events (KeyUpdate, NewSessionTicket,
/// close_notify, alert, reset), upstream close before the response head and
/// partial deliveries, no octet moves past a message boundary and zero residue
/// (legs, pipes, intents, receipts) remains after every outcome. The seed of
/// every case is printed before it runs.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn seeded_peer_events_never_move_an_octet_past_a_message_boundary_and_leave_no_residue() {
    for seed in 0..CASES {
        let mut rng = StdRng::seed_from_u64(seed);
        let ending = pick_ending(&mut rng);
        let request_body = pattern(rng.gen_range(1..=48), 7);
        let response_body = pattern(rng.gen_range(2..=64), 91);
        eprintln!("S-PIG-102 seed={seed} ending={ending:?}");

        let request =
            [request_head("POST", "/a", request_body.len()), request_body.clone()].concat();
        let mut rig = Rig::start(122, seed).await;

        let response = response_wire(200, &response_body);
        let (head_len, _) = split_message(&response).map(|(h, b)| (h.len(), b)).expect("head");
        let public;
        let upstream;
        match ending {
            Ending::Completes => {
                public = script_with_controls(
                    LegScript::new(),
                    &cut(&request, &mut rng),
                    &mut rng,
                    true,
                )
                .quiet(Duration::from_secs(1))
                .close_notify();
                upstream = script_with_controls(
                    LegScript::new(),
                    &cut(&response, &mut rng),
                    &mut rng,
                    true,
                );
            }
            Ending::OriginClosesBeforeHead(end) => {
                public = script_with_controls(
                    LegScript::new(),
                    &cut(&request, &mut rng),
                    &mut rng,
                    true,
                )
                .quiet(Duration::from_secs(1))
                .close_notify();
                upstream = end_with(LegScript::new(), end);
            }
            Ending::OriginFailsMidBody(end) => {
                public = script_with_controls(
                    LegScript::new(),
                    &cut(&request, &mut rng),
                    &mut rng,
                    true,
                )
                .quiet(Duration::from_secs(1))
                .close_notify();
                let partial = head_len + rng.gen_range(1..response_body.len());
                upstream = end_with(
                    script_with_controls(
                        LegScript::new(),
                        &cut(&response[..partial], &mut rng),
                        &mut rng,
                        true,
                    ),
                    end,
                );
            }
            Ending::ClientFailsMidBody(end) => {
                let partial = request.len() - rng.gen_range(1..=request_body.len());
                public = end_with(
                    script_with_controls(
                        LegScript::new(),
                        &cut(&request[..partial], &mut rng),
                        &mut rng,
                        true,
                    ),
                    end,
                );
                upstream = script_with_controls(
                    LegScript::new(),
                    &cut(&response, &mut rng),
                    &mut rng,
                    false,
                );
            }
        }
        let (first, client) = rig.start_exchange(public, vec![upstream]);
        let legs = rig.finish(first).await;
        drop(client);

        let public_leg = legs.iter().find(|leg| leg.role == LegRole::Public).expect("public leg");
        let upstream_legs: Vec<&LegTranscript> =
            legs.iter().filter(|leg| leg.role == LegRole::Upstream).collect();
        assert!(upstream_legs.len() <= 1, "seed={seed}: one request, one origin connection");

        // Request side: whatever reached the origin is exactly one head and a
        // prefix of the request body; nothing past the request boundary.
        for leg in &upstream_legs {
            if let Some((head, body)) = split_message(&leg.written) {
                assert!(
                    request_body.starts_with(body),
                    "seed={seed}: origin received octets that are not the request body prefix"
                );
                assert!(head.starts_with(b"POST /a HTTP/1.1\r\n"), "seed={seed}: origin head");
                if !matches!(ending, Ending::ClientFailsMidBody(_)) {
                    assert_eq!(
                        body, &request_body,
                        "seed={seed}: the origin got the whole request body and nothing else"
                    );
                }
            }
        }

        // Response side.
        match ending {
            Ending::Completes => {
                let (head, body) = split_message(&public_leg.written).expect("response head");
                assert_eq!(status_of(head), Some(200), "seed={seed}: status");
                assert_eq!(body, response_body, "seed={seed}: octet-exact response body");
            }
            Ending::OriginClosesBeforeHead(_) => {
                let (head, rest) = split_message(&public_leg.written).expect("a 502 head");
                assert_eq!(status_of(head), Some(502), "seed={seed}: exactly one 502");
                // The whole public-leg stream is that one response: no second
                // status line anywhere, and no stray octets after its body.
                let status_lines = public_leg
                    .written
                    .windows(b"HTTP/1.".len())
                    .filter(|window| *window == b"HTTP/1.")
                    .count();
                assert_eq!(status_lines, 1, "seed={seed}: exactly one response on the public leg");
                let declared_length = std::str::from_utf8(head).ok().and_then(|text| {
                    text.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                });
                assert_eq!(
                    rest.len(),
                    declared_length.unwrap_or(0),
                    "seed={seed}: no octets after the 502 beyond its declared body"
                );
                assert!(
                    head.starts_with(b"HTTP/1.1 502"),
                    "seed={seed}: the response is exactly a 502 status line, not a success head"
                );
            }
            Ending::OriginFailsMidBody(_) => {
                let (head, body) = split_message(&public_leg.written)
                    .expect("the response head was already relayed before the origin failed");
                assert_eq!(status_of(head), Some(200), "seed={seed}: head already sent");
                assert!(
                    response_body.starts_with(body) && body.len() < response_body.len(),
                    "seed={seed}: only a strict prefix of the response body is relayed"
                );
                assert!(
                    !public_leg.close_notify_sent,
                    "seed={seed}: an unclean end sends no close_notify after the head"
                );
            }
            Ending::ClientFailsMidBody(_) => {
                assert!(
                    public_leg.written.is_empty(),
                    "seed={seed}: a request that fails mid-body gets no response on the failed leg"
                );
            }
        }

        let report = rig.shutdown().await;
        assert_eq!(report.forced_connections, 0, "seed={seed}: nothing is left to force");
        wait_until("every leg to be closed", || rig.relay.live_leg_count() == 0).await;
        rig.assert_no_residue(&format!("seed={seed} {ending:?}"));
    }
}

// ---------------------------------------------------------------------------
// S-PIG-103
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    RequestHead,
    RequestBody,
    ResponseBody,
    Discard,
}

fn pick_phase(rng: &mut StdRng) -> Phase {
    match rng.gen_range(0..4_u8) {
        0 => Phase::RequestHead,
        1 => Phase::RequestBody,
        2 => Phase::ResponseBody,
        _ => Phase::Discard,
    }
}

/// One drain run: the ordered observation of everything the gateway did, built
/// only from the Sim transcripts and the shutdown report.
async fn drain_case(seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let phase = pick_phase(&mut rng);
    let finishes = rng.gen_bool(0.5);
    let queue_second = rng.gen_bool(0.5);
    eprintln!("S-PIG-103 seed={seed} phase={phase:?} finishes_in_time={finishes}");
    let stall = if finishes { Duration::from_secs(1) } else { Duration::from_secs(3600) };

    let mut rig = Rig::start(123, seed).await;
    let second = request_head("GET", "/second", 0);
    let (public, upstream, ready): (LegScript, Vec<LegScript>, fn(&[LegTranscript]) -> bool) =
        match phase {
            Phase::RequestHead => (
                LegScript::new()
                    .octets(b"GET /slow HTTP/1.1\r\nHost: api.exa".to_vec())
                    .quiet(stall)
                    .octets(b"mple.com\r\n\r\n".to_vec())
                    .close_notify(),
                vec![],
                |legs| legs.iter().any(|leg| leg.role == LegRole::Public),
            ),
            Phase::RequestBody => (
                LegScript::new()
                    .octets(request_head("POST", "/slow", 10))
                    .octets(b"hello".to_vec())
                    .quiet(stall)
                    .octets(b"world".to_vec())
                    .close_notify(),
                vec![LegScript::new().octets(response_wire(200, b"done"))],
                |legs| {
                    legs.iter().any(|leg| leg.role == LegRole::Upstream && !leg.written.is_empty())
                },
            ),
            Phase::ResponseBody => {
                let mut public = LegScript::new().octets(request_head("GET", "/slow", 0));
                if queue_second {
                    public = public.octets(second.clone());
                }
                (
                    public.close_notify(),
                    vec![
                        LegScript::new()
                            .octets(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nhalf".to_vec())
                            .quiet(stall)
                            .octets(b"done".to_vec()),
                    ],
                    |legs| {
                        legs.iter().any(|leg| {
                            leg.role == LegRole::Public
                                && leg.written.windows(12).any(|w| w == b"HTTP/1.1 200")
                        })
                    },
                )
            }
            Phase::Discard => (
                LegScript::new()
                    .octets(request_head("POST", "/a", 100_000))
                    .octets(vec![b'x'; 1000])
                    .quiet(stall),
                vec![LegScript::new().octets(response_wire(401, b"no"))],
                |legs| {
                    legs.iter().any(|leg| {
                        leg.role == LegRole::Public
                            && leg.calls.contains(&RelayCall::DiscardRemaining)
                    })
                },
            ),
        };
    let (first, client) = rig.start_exchange(public, upstream);
    wait_until("the seeded phase to be reached", || ready(&rig.legs_since(first))).await;

    // Drain begins here.
    let report = rig.shutdown().await;
    let legs = rig.finish(first).await;
    drop(client);

    let upstream_legs = legs.iter().filter(|leg| leg.role == LegRole::Upstream).count();
    assert!(
        upstream_legs <= 1,
        "seed={seed}: a connection was reused or a later request was parsed after the drain began"
    );
    let discarding = legs
        .iter()
        .any(|leg| leg.role == LegRole::Public && leg.calls.contains(&RelayCall::DiscardRemaining));
    if finishes {
        assert_eq!(
            report.forced_connections, 0,
            "seed={seed}: the exchange completed in the drain"
        );
    } else if discarding {
        // Lingering close (R-14, R-64): a discard in progress ends at the drain
        // deadline as an ordinary bounded end, not a force-abort; the socket is
        // then closed (the every-leg-closed and no-residue checks below). The
        // request-body phase reaches this state too, because its scripted origin
        // answers before the body completes.
        assert_eq!(
            report.forced_connections, 0,
            "seed={seed}: a stalled discard ends at the drain deadline unforced"
        );
    } else {
        assert_eq!(
            report.forced_connections, 1,
            "seed={seed}: the stalled exchange was force-aborted at the one drain deadline"
        );
    }
    wait_until("every leg to be closed", || rig.relay.live_leg_count() == 0).await;
    rig.assert_no_residue(&format!("seed={seed} {phase:?}"));

    // The ordered observation: per leg, role, the ordered port calls, the octets
    // written, close_notify and closure; then the report's projection.
    let mut trace: Vec<String> = legs
        .iter()
        .map(|leg| {
            format!(
                "{:?} calls={:?} written={} close_notify={} closed={}",
                leg.role,
                leg.calls,
                String::from_utf8_lossy(&leg.written).replace("\r\n", "|"),
                leg.close_notify_sent,
                leg.closed
            )
        })
        .collect();
    trace.push(format!(
        "report forced={} clean={} identity_empty={}",
        report.forced_connections,
        report.is_clean(),
        report.identity_empty
    ));
    trace
}

/// S-PIG-103: drain beginning at a seeded point of an exchange (request head,
/// request body, response body, discard) completes the exchange or force-aborts
/// it at the one drain deadline (a stalled discard instead ends there unforced), never reuses a connection or parses a later
/// request, and leaves no residue; twin runs at one seed produce the same
/// ordered trace. The seed of every case is printed before it runs.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn drain_at_a_seeded_point_completes_or_force_aborts_and_twin_runs_agree() {
    for seed in 0..CASES {
        // Twin runs: the same seeded case composed twice in sequence (the first
        // composition is shut down before the second binds the same address).
        let first = drain_case(seed).await;
        let twin = drain_case(seed).await;
        assert_eq!(first, twin, "seed={seed}: twin runs produced different ordered traces");
    }
}
