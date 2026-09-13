//! Real-rustls exact-peer failure for `HostGatewayClientMtls`.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::manual_let_else,
    reason = "real-rustls acceptance fixture preconditions and Contract Shape metadata"
)]

use std::io::BufReader;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::gateway_identity::GatewayIdentityEpoch;
use overdrive_core::id::{CertSerial, ContentHash, NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{GatewayFrontendDemandWake, PublicCertifiedKeyId};
use overdrive_core::traits::Clock;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_gateway::application::UnboundGatewayBuilder;
use overdrive_gateway::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayIdentityActionTarget,
    GatewayIdentityMutationOutcome, GatewayUpstreamSeal,
};
use overdrive_gateway::runtime::GatewayLimits;

use super::helpers::mtls_pki::{Leaf, TestPki};

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER HostGatewayClientMtls exact-peer adapter"]
async fn valid_selected_peer_observes_the_dedicated_gateway_svid_client_identity() {
    let pki = TestPki::mint();
    let host = host_adapter(&pki).await;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("peer listener");
    let address = listener.local_addr().expect("peer address");
    let server_config = server_config_for_peer(&pki, &pki.gateway_peer_a);
    let (identity_tx, identity_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("one connector attempt");
        let mut tls = tokio_rustls::TlsAcceptor::from(server_config)
            .accept(socket)
            .await
            .expect("valid exact-peer handshake");
        let presented = presented_spiffe(tls.get_ref().1.peer_certificates());
        let _ = identity_tx.send(presented);
        let mut byte = [0_u8; 1];
        assert_eq!(tokio::io::AsyncReadExt::read(&mut tls, &mut byte).await.unwrap_or(0), 1);
    });

    let socket = tokio::net::TcpStream::connect(address).await.expect("connected fd");
    let owned: OwnedFd = socket.into_std().expect("std socket").into();
    let mut upstream = host
        .authenticate(
            owned,
            pki.gateway_peer_a.spiffe.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .await
        .expect("exact peer A authenticates");
    tokio::io::AsyncWriteExt::write_all(&mut upstream, b"x")
        .await
        .expect("one authenticated body byte");
    assert_eq!(
        identity_rx.await.expect("peer identity observation"),
        Some(pki.gateway_client.spiffe.clone()),
    );
    server.await.expect("peer task");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER HostGatewayClientMtls exact-peer adapter"]
async fn selected_identity_a_rejects_valid_peer_b_before_body_without_retry_or_fallback() {
    let pki = TestPki::mint();
    let host = host_adapter(&pki).await;

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("peer listener");
    let address = listener.local_addr().expect("peer address");
    let accepts = Arc::new(AtomicU32::new(0));
    let body_bytes = Arc::new(AtomicU32::new(0));
    let server_config = server_config_for_peer(&pki, &pki.gateway_peer_b);
    let server_accepts = Arc::clone(&accepts);
    let server_body = Arc::clone(&body_bytes);
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("one connector attempt");
        server_accepts.fetch_add(1, Ordering::SeqCst);
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
        if let Ok(mut tls) = acceptor.accept(socket).await {
            let mut byte = [0_u8; 1];
            if tokio::io::AsyncReadExt::read(&mut tls, &mut byte).await.unwrap_or(0) > 0 {
                server_body.fetch_add(1, Ordering::SeqCst);
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err(),
            "wrong-peer failure must not open an alternate/retry connection",
        );
    });

    let socket = tokio::net::TcpStream::connect(address).await.expect("connected fd");
    let owned: OwnedFd = socket.into_std().expect("std socket").into();
    let error = match host
        .authenticate(
            owned,
            pki.gateway_peer_a.spiffe.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .await
    {
        Ok(_) => panic!("valid peer B must not satisfy selected identity A"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        GatewayClientMtlsError::PeerSpiffeMismatch { expected, actual }
            if expected == pki.gateway_peer_a.spiffe && actual == pki.gateway_peer_b.spiffe
    ));
    server.await.expect("peer task");
    assert_eq!(accepts.load(Ordering::SeqCst), 1);
    assert_eq!(body_bytes.load(Ordering::SeqCst), 0, "failure precedes HTTP body bytes");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER HostGatewayClientMtls identity gate"]
async fn absent_expired_and_malformed_gateway_identities_fail_before_tls_or_body_bytes() {
    let pki = TestPki::mint();
    for (material, expected) in [
        (None, GatewayClientMtlsError::IdentityAbsent),
        (Some(expired_gateway_material(&pki)), GatewayClientMtlsError::IdentityExpired),
        (Some(malformed_gateway_material(&pki)), GatewayClientMtlsError::Handshake),
    ] {
        let host = host_adapter_with_material(&pki, material).await;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("identity-gate peer listener");
        let address = listener.local_addr().expect("peer address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("one connected fd");
            let mut observed = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut socket, &mut observed)
                .await
                .expect("capture bytes before close");
            observed
        });
        let socket = tokio::net::TcpStream::connect(address).await.expect("connected fd");
        let owned: OwnedFd = socket.into_std().expect("std socket").into();
        let result = host
            .authenticate(
                owned,
                pki.gateway_peer_a.spiffe.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert!(matches!(result, Err(error) if error == expected));
        assert!(server.await.expect("capture task").is_empty(), "identity gate precedes TLS bytes");
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER HostGatewayClientMtls peer-address and deadline mapping"]
async fn non_peer_fd_and_expired_handshake_deadline_map_to_exact_closed_errors() {
    let pki = TestPki::mint();
    let host = host_adapter(&pki).await;
    let not_a_socket: OwnedFd = std::fs::File::open("/dev/null").expect("non-socket fd").into();
    let peer_address = host
        .authenticate(
            not_a_socket,
            pki.gateway_peer_a.spiffe.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .await;
    assert!(matches!(peer_address, Err(GatewayClientMtlsError::PeerAddress)));

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("stalled peer listener");
    let address = listener.local_addr().expect("peer address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("one connector attempt");
        let mut observed = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut socket, &mut observed)
            .await
            .expect("capture stalled handshake bytes");
        observed
    });
    let socket = tokio::net::TcpStream::connect(address).await.expect("connected fd");
    let owned: OwnedFd = socket.into_std().expect("std socket").into();
    let timeout = host.authenticate(owned, pki.gateway_peer_a.spiffe.clone(), Instant::now()).await;
    assert!(matches!(timeout, Err(GatewayClientMtlsError::HandshakeTimeout)));
    let observed = server.await.expect("stalled peer task");
    assert!(
        !observed.windows(b"GET /".len()).any(|window| window == b"GET /"),
        "handshake timeout cannot fall back to plaintext HTTP",
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER HostGatewayClientMtls peer SPIFFE shape validation"]
async fn trusted_peers_with_zero_or_multiple_uri_sans_fail_as_peer_spiffe_shape() {
    let pki = TestPki::mint();
    for malformed_peer in [&pki.gateway_peer_without_spiffe, &pki.gateway_peer_with_two_spiffes] {
        let host = host_adapter(&pki).await;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("malformed-shape peer listener");
        let address = listener.local_addr().expect("peer address");
        let server_config = server_config_for_peer(&pki, malformed_peer);
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("one connector attempt");
            let _ = tokio_rustls::TlsAcceptor::from(server_config).accept(socket).await;
        });
        let socket = tokio::net::TcpStream::connect(address).await.expect("connected fd");
        let owned: OwnedFd = socket.into_std().expect("std socket").into();
        let result = host
            .authenticate(
                owned,
                pki.gateway_peer_a.spiffe.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert!(matches!(result, Err(GatewayClientMtlsError::PeerSpiffeShape)));
        server.await.expect("malformed-shape peer task");
    }
}

struct NoopWake;
impl GatewayFrontendDemandWake for NoopWake {
    fn wake(&self, _service_id: ServiceId) {}
}

async fn host_adapter(
    pki: &TestPki,
) -> overdrive_dataplane::mtls::gateway_identity::HostGatewayClientMtls {
    host_adapter_with_material(pki, Some(pki.gateway_client_svid_material())).await
}

async fn host_adapter_with_material(
    pki: &TestPki,
    material: Option<SvidMaterial>,
) -> overdrive_dataplane::mtls::gateway_identity::HostGatewayClientMtls {
    let slot = Arc::new(overdrive_dataplane::mtls::gateway_identity::GatewayIdentitySlot::new(
        NodeId::new("gateway-node").expect("node ID"),
        pki.trust_bundle(),
    ));
    if let Some(material) = material {
        assert_eq!(
            slot.action_target().hold_audited(GatewayIdentityEpoch::first(), material),
            GatewayIdentityMutationOutcome::Applied,
        );
    }
    overdrive_dataplane::mtls::gateway_identity::HostGatewayClientMtls::new(
        slot.client_access(Arc::new(overdrive_sim::adapters::clock::SimClock::new())),
        gateway_seal().await,
        GatewayLimits::first_slice(),
    )
}

fn expired_gateway_material(pki: &TestPki) -> SvidMaterial {
    let leaf = &pki.gateway_client;
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        CertSerial::new(leaf.serial.as_str()).expect("serial"),
        SpiffeId::new(leaf.spiffe.as_str()).expect("SPIFFE ID"),
        CaKeyPem::new(leaf.key_pem.clone()),
        UnixInstant::from_unix_duration(Duration::ZERO),
    )
}

fn malformed_gateway_material(pki: &TestPki) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new("not-a-certificate".to_owned()),
        CaCertDer::new(vec![1, 2, 3]),
        CertSerial::new(pki.gateway_client.serial.as_str()).expect("serial"),
        SpiffeId::new(pki.gateway_client.spiffe.as_str()).expect("SPIFFE ID"),
        CaKeyPem::new("not-a-private-key".to_owned()),
        UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)),
    )
}

async fn gateway_seal() -> GatewayUpstreamSeal {
    let root = tempfile::tempdir().expect("gateway seal composition root");
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("public key");
    let cert = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
        .expect("public certificate params")
        .self_signed(&key)
        .expect("public certificate");
    let chain = root.path().join("public-chain.pem");
    let private_key = root.path().join("public-key.pem");
    std::fs::write(&chain, cert.pem()).expect("chain");
    std::fs::write(&private_key, key.serialize_pem()).expect("key");
    std::fs::set_permissions(&private_key, std::fs::Permissions::from_mode(0o600))
        .expect("private-key mode");
    assert_eq!(
        std::fs::metadata(&private_key).expect("key metadata").permissions().mode() & 0o777,
        0o600,
        "GatewayConfig fixture requires exact private-key mode",
    );
    let config = overdrive_gateway::GatewayConfig::new(
        std::net::Ipv4Addr::LOCALHOST,
        overdrive_gateway::ManualCertifiedKeyConfig::new(
            PublicCertifiedKeyId::new("api-origin").expect("key ID"),
            chain,
            private_key,
        ),
    );
    let intent = Arc::new(
        overdrive_store_local::LocalIntentStore::open(root.path().join("intent.redb"))
            .expect("intent store"),
    );
    let observations =
        Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
            NodeId::new("gateway-node").expect("node ID"),
            54,
        ));
    let vip_view = Arc::new(overdrive_sim::adapters::read_ports::SimServiceVipView::new(
        std::collections::BTreeMap::<ContentHash, ServiceVip>::new(),
    ));
    let clock: Arc<dyn Clock> = Arc::new(overdrive_sim::adapters::clock::SimClock::new());
    let builder = UnboundGatewayBuilder::new(
        config,
        intent,
        observations,
        vip_view,
        Arc::new(overdrive_host::ca::PublicCertifiedKeyAeadCodec::new(Arc::new(
            overdrive_sim::adapters::SimKek::for_boot(),
        ))),
        Arc::new(overdrive_host::socket_cookie::SocketCookieReader::new()),
        clock,
    )
    .await
    .expect("unbound gateway builder");
    let (_builder, _demand, seal) = builder.bind_demand_wake(Arc::new(NoopWake));
    seal
}

fn server_config_for_peer(pki: &TestPki, peer: &Leaf) -> Arc<rustls::ServerConfig> {
    let mut roots = rustls::RootCertStore::empty();
    let mut root_reader = BufReader::new(pki.ca_cert_pem().as_bytes());
    for cert in rustls_pemfile::certs(&mut root_reader) {
        roots.add(cert.expect("root DER")).expect("root trust anchor");
    }
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .expect("gateway client verifier");
    Arc::new(
        rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![peer.cert_der.clone(), pki.intermediate_cert_der()],
                peer.key_der.clone_key(),
            )
            .expect("valid selected-peer server config"),
    )
}

fn presented_spiffe(
    certs: Option<&[rustls::pki_types::CertificateDer<'_>]>,
) -> Option<overdrive_core::id::SpiffeId> {
    use x509_parser::prelude::FromDer as _;

    let leaf = certs?.first()?;
    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(leaf.as_ref()).ok()?;
    let san = parsed.subject_alternative_name().ok()??;
    let mut uris = san.value.general_names.iter().filter_map(|name| match name {
        x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
        _ => None,
    });
    let only = uris.next()?;
    if uris.next().is_some() {
        return None;
    }
    only.parse().ok()
}
