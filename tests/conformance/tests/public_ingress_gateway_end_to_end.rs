//! Public-ingress gateway: the production composition end to end on native
//! metal (`AT-PIG-E2E-1` and the native arms of S-PIG-02, 39, 40, 56, 60, 69,
//! 70, 71, 105, 109, 113 and 115).
//!
//! Every test starts the exported production server handler in-process
//! (`run_server(config, NodeAdapters::host(..))` over the real cgroupfs, the
//! real `EbpfDataplane`, the kernel-TLS relay and a real Cloud Hypervisor VM
//! Service) with the gateway enabled through `ServerConfig.gateway`, deploys the
//! Service and the Route through the server's public HTTPS API, and makes the
//! request as an external TLS 1.3 client on a real socket whose only trust
//! anchor is the composition's test CA. No test spawns the `overdrive` binary,
//! installs a map, rule, identity or kernel-TLS state, or seeds a receipt; the
//! only simulated edge is the hermetic KEK. Oracles are public bytes and
//! statuses, the gateway's `gateway.upstream.receipt.validated` occurrence, and
//! independent kernel reads (`/proc/net/tls_stat`, `ss -tni`, `bpftool`,
//! `/proc/self/fd`, `F_GETPIPE_SZ`, an `AF_PACKET` capture of loopback).
//!
//! # Gateway addresses (R-6)
//!
//! A composition that reaches promotion binds its own `127.<octet>.0.1:443`.
//! This file owns second octets 100..=119 (the gateway integration files own
//! 41..=62 and 68..=96, the in-crate drivers 63..=67, the boot/shutdown file 97).
//!
//! | octet | test | scenario |
//! |---|---|---|
//! | 100 | `..._streams_a_workload_response_over_kernel_tls_legs` | S-PIG-01 |
//! | 101 | `..._returns_503_without_waiting_when_the_service_never_runs` | S-PIG-02 (N) |
//! | 102 | `..._answers_404_outside_a_narrow_route_without_reaching_the_workload` | S-PIG-39 |
//! | 103 | `..._answers_502_exactly_once_when_the_workload_closes_before_responding` | S-PIG-40 |
//! | 104 | `..._never_succeeds_through_a_stopped_workloads_stale_service_entry` | S-PIG-56 (N) |
//! | 105 | `..._runs_its_stages_in_order_and_shuts_down_within_one_drain` | S-PIG-60 (N) |
//! | 106 | `..._answers_502_when_the_guest_flow_ends_mid_request` | S-PIG-69 (N) |
//! | 107 | `..._rejects_unacceptable_requests_with_their_exact_statuses` | S-PIG-70 |
//! | 108 | `..._answers_408_for_a_stalled_request_head` | S-PIG-70 |
//! | 109 | `..._answers_408_for_a_stalled_request_body` | S-PIG-70 |
//! | 110 | `..._answers_503_until_the_hydrator_reprograms_after_a_serve_restart` | S-PIG-71 (serve restart) |
//! | 111 | `..._holds_128_concurrent_exchanges_within_the_descriptor_and_memory_budget` | S-PIG-113 |
//! | 112 | `..._relays_exact_length_and_chunked_bodies_octet_exact_over_keep_alive` | S-PIG-105 (N) |
//! | 113 | `..._sends_no_key_update_across_a_sixteen_mebibyte_response` | S-PIG-109 (N) |
//! | 114 | `..._lets_the_client_read_the_response_before_a_clean_close` | S-PIG-115 (N) |
//!
//! # S-PIG-70 coverage
//!
//! The 404, 502 and 503 mappings of S-PIG-70 are asserted by the S-PIG-39,
//! S-PIG-40 and S-PIG-02 tests above (the same statuses through the same
//! listener); the tests numbered 107..=109 carry the statuses with no other
//! owner: 400, 421, 505, 413, 431 and the two 408 cases.
//!
//! # Fixtures
//!
//! S-PIG-01 serves the checked-in example guest (materialised by
//! `examples/public-ingress-gateway/prepare.sh`); every other test serves the
//! conformance guest (`tests/conformance/guest/pig_guest.rs`), the example
//! guest's superset with the silent-close, exact-length, chunked, early-answer,
//! delayed and held-stream routes the cases need.
//!
//! # Activation
//!
//! Every test carries the reason `pending DELIVER public-request-end-to-end`;
//! the native-metal selector runs them once DELIVER lands the enabled
//! composition, with the reason later replaced by
//! `native x86_64 KVM host required`.

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::significant_drop_tightening,
    reason = "conformance bodies carry CONTRACT_SHAPE markers and the whole Given/When/Then narrative"
)]

use std::time::{Duration, Instant};

use overdrive_system_conformance::TraceHistory;
use overdrive_system_conformance::gateway::{
    BODY_CEILING_OCTETS, BODY_NO_PROGRESS_DEADLINE, DRAIN_DEADLINE, FIVE_SECOND_DEADLINE,
    GatewayWorld, GuestKind, HEAD_CEILING_OCTETS, KernelSample, LoadChild, MAX_PUBLIC_CONNECTIONS,
    PIPE_CAPACITY_OCTETS, Request, RequestBody, RouteExtent, ServiceShape, StreamEnd, Suite,
    TlsStat, WireCapture, blocking, first_pattern_mismatch, fnv1a, kernel_sample, kill_vmm,
    live_gateway_counts, nofile_limit, open_fds, pattern_bytes, pipe_capacity_octets,
    receipt_occurrences, rss_kib, run_load_child_if_requested, service_map_entries,
    shutdown_stage_events, startup_refusal_reasons, tls_record_types,
};
use rustls::CipherSuite;

/// The streamed body the example guest writes for `GET /stream`.
fn stream_body() -> Vec<u8> {
    (0..8).flat_map(|index| format!("PIG-STREAM-CHUNK-{index:02}\n").into_bytes()).collect()
}

// ---------------------------------------------------------------------------
// S-PIG-01 -- the walking skeleton
// ---------------------------------------------------------------------------

/// S-PIG-01 (`AT-PIG-E2E-1`) -- An external user receives the api workload's
/// streamed response through the public Route; both gateway legs are
/// kernel-TLS armed while the body streams, and everything converges on cleanup.
///
/// Given the production composition has a complete manual Public Certified Key
///   and the operator has deployed the checked-in api VM Service and the
///   public-api Route
/// When an external user requests `https://api.example.com/stream` over TLS 1.3
/// Then the user receives 200, `X-Workload: api` and the streamed body
///  And the receipt occurrence shows a Selected backend whose applied identity
///      is the api allocation
///  And the leg-B/leg-C segment carries TLS 1.3 records with no body cleartext
///  And the public leg and the upstream leg are kernel-TLS armed while it streams
///  And Route, Service, gateway intent, receipt and identity state converge.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_streams_a_workload_response_over_kernel_tls_legs()
{
    let trace = TraceHistory::install_global();
    let world = GatewayWorld::serving(&trace, 100, GuestKind::Example, ServiceShape::Serving).await;
    let address = world.gateway_address();

    // The relay probe and the connect probe ran before the bind: when the
    // listener is first observed bound, the probe occurrence is already
    // recorded and no gateway refusal was emitted.
    world.wait_listener_bound(Duration::from_secs(30)).await;
    let before_bind = trace.snapshot();
    assert!(
        !receipt_occurrences(&before_bind, "probe").is_empty(),
        "the connect-probe occurrence precedes the first observed bind"
    );
    assert!(
        startup_refusal_reasons(&before_bind).iter().all(|reason| !reason.starts_with("gateway.")),
        "no gateway gate refused: {:?}",
        startup_refusal_reasons(&before_bind)
    );

    // Baseline kernel state, then the request with a loopback capture running.
    let baseline = blocking(move || kernel_sample(address)).await;
    let capture = blocking(WireCapture::start).await;
    let mark = trace.mark();
    let client = world.client();
    let (head_tx, head_rx) = tokio::sync::oneshot::channel::<Duration>();
    let request = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let mut connection = client.connect().expect("TLS 1.3 handshake with the gateway");
        connection.send(&Request::get("/stream").encode()).expect("send the request");
        let head = connection.read_head().expect("the response head");
        let head_at = started.elapsed();
        let _ = head_tx.send(head_at);
        let (body, chunks, _) = connection.read_body(&head, false).expect("the streamed body");
        (head, body, chunks, head_at, started.elapsed())
    });

    // While the body streams: three samples, one second apart, starting once
    // the head has reached the client.
    let head_at = head_rx.await.expect("the head arrives");
    let mut streaming = Vec::<KernelSample>::new();
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        streaming.push(blocking(move || kernel_sample(address)).await);
    }
    let (head, body, chunks, _, total) = request.await.expect("the request task completes");

    // Then: status, header and the streamed body.
    assert_eq!(head.status, 200);
    assert_eq!(head.header("x-workload"), Some("api"));
    assert_eq!(body, stream_body(), "the streamed body arrives octet-exact");
    assert!(chunks.len() > 1, "the body is relayed chunk by chunk: {chunks:?}");
    assert!(
        head_at + Duration::from_secs(1) < total,
        "the head is delivered while the body is still streaming ({head_at:?} of {total:?})"
    );

    // And: the receipt occurrence names a Selected backend whose applied
    // identity is the api allocation.
    let events = trace.since(mark);
    let occurrences = receipt_occurrences(&events, "request");
    assert_eq!(occurrences.len(), 1, "exactly one receipt occurrence for the request");
    let receipt = occurrences[0];
    assert_eq!(receipt.fields.get("receipt_outcome").map(String::as_str), Some("selected"));
    assert!(receipt.fields.contains_key("backend_id"), "a Selected receipt names its backend");
    let allocation = world.allocation_id().await.expect("the api allocation exists");
    assert!(
        receipt
            .fields
            .get("expected_peer_spiffe_id")
            .is_some_and(|identity| identity.contains(&allocation)),
        "the applied identity is the api allocation {allocation}: {:?}",
        receipt.fields
    );

    // And: both gateway legs are kernel-TLS armed while the body streams.
    for (index, sample) in streaming.iter().enumerate() {
        assert!(sample.public_armed >= 1, "sample {index}: the public leg is armed: {sample:?}");
        assert!(
            sample.upstream_armed >= 2,
            "sample {index}: the gateway's upstream leg and its leg-C peer are armed: {sample:?}"
        );
        assert!(
            sample.tls.delta(&baseline.tls, "TlsCurrTxSw") >= 2
                && sample.tls.delta(&baseline.tls, "TlsCurrRxSw") >= 2,
            "sample {index}: kernel TLS TX and RX are installed on both legs"
        );
        assert_eq!(
            sample.tls.delta(&baseline.tls, "TlsDecryptError"),
            0,
            "sample {index}: no record failed to decrypt"
        );
    }

    // And: the leg-B/leg-C segment carried TLS records and no body cleartext.
    let evidence = blocking(move || capture.finish()).await;
    let inner = evidence.segments_excluding_ports(&[443]);
    assert!(
        tls_record_types(&inner).contains(&0x17),
        "the inter-agent segment carries TLS application-data records"
    );
    assert!(
        !evidence.any_payload_contains(b"PIG-STREAM-CHUNK-"),
        "no body cleartext appears on any loopback segment"
    );

    // And: the armed legs close with the exchange.
    blocking(move || {
        overdrive_system_conformance::wait_until(
            Duration::from_secs(10),
            "the kernel-TLS legs of the finished request close",
            || kernel_sample(address).upstream_armed == 0,
        );
    })
    .await;
    let after = blocking(move || kernel_sample(address)).await;
    assert_eq!(after.tls.delta(&baseline.tls, "TlsCurrTxSw"), 0, "no armed leg remains");
    assert_eq!(after.tls.delta(&baseline.tls, "TlsCurrRxSw"), 0, "no armed leg remains");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-02 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-02 (N arm) -- A sound listener whose Service never reaches Running
/// returns 503 with no fallback, bounded by the upstream connect deadline, and
/// leaves no intent or receipt; asked twice it answers identically.
///
/// Given the production composition with the Route declared and a Service whose
///   VM never reaches Running (its command does not exist)
/// When an external user requests `/stream` twice
/// Then each answer is 503 inside the upstream connect deadline with no workload
///      content, the two bodies are identical, the receipt occurrence is
///      NoBackend with the selected-only fields absent, and the live counts are 0.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_returns_503_without_waiting_when_the_service_never_runs()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 101, GuestKind::Conformance, ServiceShape::NeverRuns).await;
    world.wait_listener_bound(Duration::from_secs(30)).await;

    let mut bodies = Vec::new();
    for round in 0..2 {
        let mark = trace.mark();
        let client = world.client();
        let (response, elapsed) = blocking(move || {
            let started = Instant::now();
            let response = client
                .connect()
                .expect("TLS 1.3 handshake")
                .exchange(&Request::get("/stream"))
                .expect("a 503 response");
            (response, started.elapsed())
        })
        .await;
        assert_eq!(response.status(), 503, "round {round}: no selectable backend is a 503");
        assert!(
            elapsed < FIVE_SECOND_DEADLINE,
            "round {round}: the refusal is typed, never a wait: {elapsed:?}"
        );
        assert!(
            !String::from_utf8_lossy(&response.body).contains("PIG-"),
            "round {round}: no fallback content"
        );
        let events = trace.since(mark);
        let occurrences = receipt_occurrences(&events, "request");
        assert_eq!(occurrences.len(), 1, "round {round}: one receipt occurrence");
        assert_eq!(
            occurrences[0].fields.get("receipt_outcome").map(String::as_str),
            Some("no_backend")
        );
        assert!(
            !occurrences[0].fields.contains_key("backend_id")
                && !occurrences[0].fields.contains_key("expected_peer_spiffe_id"),
            "round {round}: selected-only fields are structurally absent"
        );
        assert_eq!(blocking(live_gateway_counts).await, (0, 0), "round {round}: no residue");
        bodies.push(response.body);
    }
    assert_eq!(bodies[0], bodies[1], "the two unavailable answers are identical");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-39
// ---------------------------------------------------------------------------

/// S-PIG-39 -- A request outside a narrow replacement Route is a 404 and never
/// reaches the workload.
///
/// Given a serving workload whose request-count marker is 0
///  And the Route replaced by one that matches `/stream` exactly
/// When an external user requests `/exact`
/// Then the answer is 404 inside the deadline and the marker is still 0.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_404_outside_a_narrow_route_without_reaching_the_workload()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 102, GuestKind::Conformance, ServiceShape::Serving).await;
    assert_eq!(world.workload_request_count().await, 0, "no counted request has reached the guest");

    world.declare_route(RouteExtent::StreamOnly).await;
    // The replacement is current once a path outside it stops being served.
    // `/ready` is not counted by the guest, so the wait cannot move the marker.
    let client = world.client();
    let waiting = client.clone();
    blocking(move || {
        overdrive_system_conformance::wait_until(
            Duration::from_secs(30),
            "the narrow Route becomes the current application",
            || {
                waiting
                    .connect()
                    .and_then(|mut c| {
                        c.exchange(&Request::get("/ready"))
                            .map_err(|e| std::io::Error::other(format!("{e:?}")))
                    })
                    .is_ok_and(|response| response.status() == 404)
            },
        );
    })
    .await;

    let (response, elapsed) = blocking(move || {
        let started = Instant::now();
        let response = client
            .connect()
            .expect("TLS 1.3 handshake")
            .exchange(&Request::get("/exact"))
            .expect("a 404 response");
        (response, started.elapsed())
    })
    .await;
    assert_eq!(response.status(), 404);
    assert!(elapsed < FIVE_SECOND_DEADLINE, "the 404 is immediate: {elapsed:?}");

    world.declare_route(RouteExtent::WholeHost).await;
    blocking({
        let client = world.client();
        move || {
            overdrive_system_conformance::wait_until(
                Duration::from_secs(30),
                "the whole-host Route is current again",
                || {
                    client
                        .connect()
                        .and_then(|mut c| {
                            c.exchange(&Request::get("/ready"))
                                .map_err(|e| std::io::Error::other(format!("{e:?}")))
                        })
                        .is_ok_and(|response| response.status() == 204)
                },
            );
        }
    })
    .await;
    assert_eq!(
        world.workload_request_count().await,
        0,
        "the request outside the Route never reached the workload"
    );

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-40
// ---------------------------------------------------------------------------

/// S-PIG-40 -- A selected upstream that closes before any response octet maps
/// to a 502 exactly once, with a single upstream attempt.
///
/// Given a serving workload whose request-count marker is 0
/// When an external user requests `/close` (the guest closes without answering)
/// Then the user receives exactly one 502 inside the response-head deadline, the
///      connection delivers nothing further, and the marker is exactly 1.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_502_exactly_once_when_the_workload_closes_before_responding()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 103, GuestKind::Conformance, ServiceShape::Serving).await;
    assert_eq!(world.workload_request_count().await, 0);

    let client = world.client();
    let (response, trailing, elapsed) = blocking(move || {
        let started = Instant::now();
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        let response = connection.exchange(&Request::get("/close")).expect("a 502 response");
        let elapsed = started.elapsed();
        connection.set_read_timeout(Some(Duration::from_secs(2)));
        let (_, trailing) = connection.read_to_end(Duration::from_secs(5));
        (response, trailing, elapsed)
    })
    .await;
    assert_eq!(response.status(), 502, "an upstream that closes before responding is a 502");
    assert!(elapsed <= FIVE_SECOND_DEADLINE, "the 502 arrives inside the response-head deadline");
    assert_eq!(trailing, 0, "exactly one response: nothing follows the 502");
    assert_eq!(
        world.workload_request_count().await,
        1,
        "the workload saw the request exactly once: no retry, no second attempt"
    );
    assert_eq!(blocking(live_gateway_counts).await, (0, 0), "no intent or receipt remains");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-56 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-56 (N arm) -- After `job stop` the Service-map entry lingers until the
/// hydrator removes it; no request ever reaches a wrong peer: while the entry
/// stands the dial fails with exactly a 502, and once it is gone the answer is 503.
///
/// Given a serving workload
/// When the Service is stopped and a request is made every 250 ms for at most
///      10 s, until the Service map's outer entry is gone, then five more
/// Then no answer is ever a success, an answer given while the entry stood both
///      before and after the request is exactly 502, an answer given with the
///      entry absent both before and after is exactly 503, and the five final
///      answers are 503.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_never_succeeds_through_a_stopped_workloads_stale_service_entry()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 104, GuestKind::Conformance, ServiceShape::Serving).await;
    assert!(blocking(service_map_entries).await >= 1, "the Service is programmed while serving");

    world.stop_service().await;
    let client = world.client();
    let mut observed = Vec::<(bool, u16, bool)>::new();
    let window = Instant::now();
    while window.elapsed() < Duration::from_secs(10) {
        let before = blocking(service_map_entries).await > 0;
        let probe = client.clone();
        let status = blocking(move || {
            probe
                .connect()
                .expect("TLS 1.3 handshake")
                .exchange(&Request::get("/exact"))
                .map_or(0, |response| response.status())
        })
        .await;
        let after = blocking(service_map_entries).await > 0;
        observed.push((before, status, after));
        if !after {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for (before, status, after) in &observed {
        assert!(
            !matches!(status, 200..=299),
            "never a wrong-peer success after the stop: {observed:?}"
        );
        match (before, after) {
            (true, true) => assert_eq!(*status, 502, "a stale entry fails the dial: {observed:?}"),
            (false, false) => {
                assert_eq!(*status, 503, "no entry is no backend: {observed:?}");
            }
            _ => assert!(
                matches!(status, 502 | 503),
                "an answer across the removal is 502 or 503: {observed:?}"
            ),
        }
    }
    assert!(
        observed.iter().any(|(_, _, after)| !after),
        "the hydrator removed the entry within the 10 s window: {observed:?}"
    );
    for round in 0..5 {
        let probe = client.clone();
        let status = blocking(move || {
            probe
                .connect()
                .expect("TLS 1.3 handshake")
                .exchange(&Request::get("/exact"))
                .map_or(0, |response| response.status())
        })
        .await;
        assert_eq!(status, 503, "round {round}: with the entry gone the answer is 503");
    }

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-60 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-60 (N arm) -- A graceful run boots the gateway gates in order and shuts
/// the gateway stage down before the leg-C worker stage, inside the one drain.
///
/// Given the production composition serving one request
/// When it shuts down gracefully with the 5 s drain
/// Then the gateway reconciler was registered, the connect probe preceded the
///      bind, status shows identity Current and the connect path Ready, the
///      stage `gateway` begins and completes before `mtls_worker_owner` begins,
///      exactly one drain is begun with a 5 s deadline, and the shutdown fits it.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_runs_its_stages_in_order_and_shuts_down_within_one_drain()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 105, GuestKind::Conformance, ServiceShape::Serving).await;
    world.wait_listener_bound(Duration::from_secs(30)).await;

    // Boot: the gateway gates ran in order. G0 registered the reconciler, the
    // gate task ran the probes before the bind, identity became Current and the
    // connect path Ready.
    let cluster: serde_json::Value = world
        .api()
        .get("/v1/cluster/info")
        .await
        .expect("GET /v1/cluster/info")
        .json()
        .await
        .expect("cluster info body");
    assert!(
        cluster["reconcilers"]
            .as_array()
            .is_some_and(|names| names.iter().any(|n| n == "gateway-svid-lifecycle")),
        "the gateway identity reconciler is registered: {cluster}"
    );
    let status = world.gateway_status().await;
    assert_eq!(status["enabled"], true);
    assert_eq!(status["application"]["gateway_identity"]["state"], "current");
    assert_eq!(status["application"]["connect_path"], "ready");
    assert!(
        !receipt_occurrences(&trace.snapshot(), "probe").is_empty(),
        "the connect probe ran before the bind"
    );
    assert!(startup_refusal_reasons(&trace.snapshot()).is_empty(), "no gate refused");

    // One request is served, the Route and the Service are withdrawn and
    // stopped, and the graceful shutdown with the 5 s drain runs.
    let client = world.client();
    let response = blocking(move || {
        client
            .connect()
            .expect("TLS 1.3 handshake")
            .exchange(&Request::get("/exact"))
            .expect("the request is answered")
    })
    .await;
    assert_eq!(response.status(), 200);
    world.withdraw_route().await;
    world.stop_service_and_wait().await;
    let mark = trace.mark();
    let (elapsed, result) = world.shutdown().await;
    result.expect("the graceful shutdown converges");

    let events = trace.since(mark);
    let stages = shutdown_stage_events(&events);
    let position = |stage: &str, phase: &str| {
        stages.iter().position(|(name, kind)| name == stage && *kind == phase)
    };
    let gateway_begin = position("gateway", "begin").expect("the gateway stage begins");
    let gateway_done = position("gateway", "complete").expect("the gateway stage completes");
    let worker_begin = position("mtls_worker_owner", "begin").expect("the worker stage begins");
    assert!(gateway_begin < gateway_done && gateway_done < worker_begin, "{stages:?}");
    assert_eq!(gateway_begin, 0, "the gateway is the first stage: {stages:?}");
    let begins: Vec<_> = events.iter().filter(|e| e.name == "serve.shutdown.begin").collect();
    assert_eq!(begins.len(), 1, "exactly one drain is begun");
    assert!(
        begins[0].fields.get("drain_deadline").is_some_and(|d| d.contains("5s")),
        "the one drain is 5 s: {:?}",
        begins[0].fields
    );
    assert!(
        elapsed <= DRAIN_DEADLINE + Duration::from_secs(2),
        "the shutdown fits the drain: {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// S-PIG-69 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-69 (N arm) -- When the guest flow ends mid-request (the VMM is killed
/// from the host, the production stimulus that severs the flow), the request
/// ends with exactly one 502 and no intent or receipt remains.
///
/// Given a serving workload and a request held at the response-head phase
///   (`/delay` answers after 4 s)
/// When the VMM is killed from the host while the request is in flight
/// Then the user receives exactly one 502 inside the 120 s request deadline, and
///      the live intent and receipt counts are 0.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_502_when_the_guest_flow_ends_mid_request()
{
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 106, GuestKind::Conformance, ServiceShape::Serving).await;

    let client = world.client();
    let request = tokio::task::spawn_blocking(move || {
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        connection.set_read_timeout(Some(Duration::from_secs(30)));
        let started = Instant::now();
        let outcome = connection.exchange(&Request::get("/delay"));
        let (_, trailing) = connection.read_to_end(Duration::from_secs(3));
        (outcome, trailing, started.elapsed())
    });
    // The request is in flight when the guest has counted it (the marker moves on
    // arrival, `/delay` answers 4 s later) and the user has no response head yet.
    let in_flight_deadline = Instant::now() + Duration::from_secs(3);
    while world.workload_request_count().await < 1 {
        assert!(Instant::now() < in_flight_deadline, "the guest never counted the held request");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!request.is_finished(), "no response head has reached the user before the kill");
    blocking(kill_vmm).await;

    let (outcome, trailing, elapsed) = request.await.expect("the request task completes");
    let response = outcome.expect("the user receives a response, not a bare close");
    assert_eq!(response.status(), 502, "a guest flow that ends mid-request is a 502");
    assert_eq!(trailing, 0, "exactly one response");
    assert!(elapsed < Duration::from_secs(120), "inside the whole-request deadline: {elapsed:?}");
    assert_eq!(blocking(live_gateway_counts).await, (0, 0), "no intent or receipt remains");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-70
// ---------------------------------------------------------------------------

/// S-PIG-70 -- The gateway answers requests it cannot accept with the exact
/// status for each cause, before any upstream octet: 400 (a non-origin-form
/// target; repeated `Content-Length`), 421 (Host different from SNI), 505
/// (`HTTP/1.0`), 413 (declared length above the ceiling) and 431 (a head above
/// the ceiling). The 404/502/503 mappings are S-PIG-39, 40 and 02.
///
/// Given a serving workload whose request-count marker is 0
/// When the user sends each unacceptable request on its own connection
/// Then each answer carries its exact status and the marker is still 0.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_rejects_unacceptable_requests_with_their_exact_statuses()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 107, GuestKind::Conformance, ServiceShape::Serving).await;

    let oversize_head = format!(
        "GET / HTTP/1.1\r\nHost: api.example.com\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(HEAD_CEILING_OCTETS + 1)
    );
    let cases: Vec<(&str, Vec<u8>, u16)> = vec![
        (
            "a non-origin-form target",
            b"CONNECT api.example.com:443 HTTP/1.1\r\nHost: api.example.com\r\n\r\n".to_vec(),
            400,
        ),
        (
            "a repeated Content-Length",
            b"POST /echo HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx"
                .to_vec(),
            400,
        ),
        (
            "a Host different from the SNI",
            b"GET /exact HTTP/1.1\r\nHost: other.example.com\r\n\r\n".to_vec(),
            421,
        ),
        ("an HTTP/1.0 request", b"GET /exact HTTP/1.0\r\nHost: api.example.com\r\n\r\n".to_vec(), 505),
        (
            "a declared length above the ceiling",
            Request::post("/echo", RequestBody::Declared(BODY_CEILING_OCTETS + 1)).head_bytes(),
            413,
        ),
        ("a head above the ceiling", oversize_head.into_bytes(), 431),
    ];
    for (label, bytes, expected) in cases {
        let client = world.client();
        let (status, end) = blocking(move || {
            let mut connection = client.connect().expect("TLS 1.3 handshake");
            connection.send(&bytes).expect("send the unacceptable request");
            let status = connection.read_head().map(|head| head.status);
            connection.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = connection.send_close_notify();
            let (end, _) = connection.read_to_end(Duration::from_secs(5));
            (status, end)
        })
        .await;
        assert_eq!(status.ok(), Some(expected), "{label}: the exact status reaches the client");
        assert_ne!(end, StreamEnd::Reset, "{label}: the answer is not lost to a reset");
    }
    assert_eq!(
        world.workload_request_count().await,
        0,
        "no unacceptable request reached the workload"
    );

    world.tear_down().await;
}

/// S-PIG-70 (408, stalled head) -- A client that sends no complete head gets
/// exactly one 408 when the head deadline passes, counted from the end of the
/// handshake.
///
/// Given a serving workload and an open TLS connection that has sent a partial head
/// When the client stays silent
/// Then the answer is 408 no earlier than the 5 s head deadline and within 2 s
///      after it.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_408_for_a_stalled_request_head() {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 108, GuestKind::Conformance, ServiceShape::Serving).await;

    let client = world.client();
    let (status, waited) = blocking(move || {
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        connection.send(b"GET /exact HTTP/1.1\r\nHost: api.example.com\r\n").expect("partial head");
        let started = Instant::now();
        connection.set_read_timeout(Some(FIVE_SECOND_DEADLINE + Duration::from_secs(2)));
        let status = connection.read_head().map(|head| head.status);
        (status, started.elapsed())
    })
    .await;
    assert_eq!(status.ok(), Some(408), "a head that never completes is a 408");
    assert!(
        waited + Duration::from_millis(500) >= FIVE_SECOND_DEADLINE
            && waited <= FIVE_SECOND_DEADLINE + Duration::from_secs(2),
        "the 408 arrives at the head deadline: {waited:?}"
    );
    assert_eq!(world.workload_request_count().await, 0, "no upstream octet was sent");

    world.tear_down().await;
}

/// S-PIG-70 (408, stalled body) -- A client that sends a complete head and ten
/// body octets and then nothing gets exactly one 408 after the body
/// no-progress deadline, before any response octet.
///
/// Given a serving workload and a request declaring a 1024-octet body
/// When the client sends ten body octets and then stays silent
/// Then the answer is 408 after 30 s of no progress (and within 2 s more), and
///      the workload never saw a request.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_408_for_a_stalled_request_body() {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 109, GuestKind::Conformance, ServiceShape::Serving).await;

    let client = world.client();
    let (status, waited) = blocking(move || {
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        connection
            .send(&Request::post("/echo", RequestBody::Declared(1024)).head_bytes())
            .expect("complete head");
        connection.send(&pattern_bytes(0, 10)).expect("ten body octets");
        let started = Instant::now();
        connection.set_read_timeout(Some(BODY_NO_PROGRESS_DEADLINE + Duration::from_secs(2)));
        let status = connection.read_head().map(|head| head.status);
        (status, started.elapsed())
    })
    .await;
    assert_eq!(status.ok(), Some(408), "a body that stops moving is a 408");
    assert!(
        waited + Duration::from_secs(1) >= BODY_NO_PROGRESS_DEADLINE
            && waited <= BODY_NO_PROGRESS_DEADLINE + Duration::from_secs(2),
        "the 408 arrives after the no-progress deadline: {waited:?}"
    );
    assert_eq!(
        world.workload_request_count().await,
        0,
        "the stalled request never reached the guest"
    );

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-71 (serve restart half)
// ---------------------------------------------------------------------------

/// S-PIG-71 (serve restart half) -- After a serve restart on the same data
/// directory the gateway answers 503 while the Service map has no outer entry
/// and the ordinary hydrator has not re-programmed it, then serves the backend
/// within the 60 s recovery window; the old receipts are gone.
///
/// Given a serving workload
/// When the server is stopped gracefully and started again over the same data
///      directory, and a request is made every 500 ms for at most 60 s
/// Then no receipt or intent survives the restart, every answer given while the
///      Service map shows no outer entry (before and after the request) is 503,
///      and a backend response arrives within the window.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_answers_503_until_the_hydrator_reprograms_after_a_serve_restart()
 {
    let trace = TraceHistory::install_global();
    let mut world =
        GatewayWorld::serving(&trace, 110, GuestKind::Conformance, ServiceShape::Serving).await;

    world.restart().await;
    assert_eq!(
        blocking(live_gateway_counts).await,
        (0, 0),
        "the old identity and receipts are gone with the old instance"
    );

    let recovery = Instant::now();
    let mut answers = Vec::<(bool, u16, bool)>::new();
    let mut served = false;
    while recovery.elapsed() < Duration::from_secs(60) {
        let before = blocking(service_map_entries).await > 0;
        let client = world.client();
        let status = blocking(move || {
            client
                .connect()
                .and_then(|mut c| {
                    c.exchange(&Request::get("/ready"))
                        .map_err(|e| std::io::Error::other(format!("{e:?}")))
                })
                .map_or(0, |response| response.status())
        })
        .await;
        let after = blocking(service_map_entries).await > 0;
        answers.push((before, status, after));
        if status == 204 {
            served = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    for (before, status, after) in &answers {
        if !before && !after && *status != 0 {
            assert_eq!(
                *status, 503,
                "a response given while the Service map has no outer entry is 503: {answers:?}"
            );
        }
    }
    assert!(served, "the backend answers within the 60 s recovery window: {answers:?}");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-113
// ---------------------------------------------------------------------------

/// S-PIG-113 -- 128 concurrent streaming exchanges through the production
/// composition stay within the accounted descriptor and memory budget; the 129th
/// connection is closed before any TLS octet; everything is released after drain.
///
/// Given a serving workload and 128 clients (in a child process, so every
///   observation describes the gateway's process only) each holding a
///   `GET /hold` exchange open with its response head received
/// When the process descriptor count, resident set and pipe capacity are read
///  And a 129th connection is made
/// Then descriptors stay within the baseline plus 768, resident-set growth plus
///      pipe capacity growth stays within 24 MiB (128 x 192 KiB), the 129th
///      connection is closed before any TLS octet, and after the exchanges drain
///      the descriptor count and pipe capacity return to the baseline.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_holds_128_concurrent_exchanges_within_the_descriptor_and_memory_budget()
 {
    // Re-executed as the load client: hold the exchanges and speak the line
    // protocol with the parent. A normal run does nothing here.
    run_load_child_if_requested();
    if std::env::var_os(overdrive_system_conformance::gateway::LOAD_CHILD_ENV).is_some() {
        return;
    }

    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 111, GuestKind::Conformance, ServiceShape::Serving).await;
    let address = world.gateway_address();
    let ca = world.write_ca_der();

    let (soft, hard) = nofile_limit();
    tracing::info!(soft, hard, "RLIMIT_NOFILE reported, not raised");
    let (fds_before, rss_before, pipes_before) =
        blocking(|| (open_fds(), rss_kib(), pipe_capacity_octets())).await;

    let mut load = LoadChild::spawn(
        "native_metal_kvm_public_ingress_gateway_holds_128_concurrent_exchanges_within_the_descriptor_and_memory_budget",
        address,
        MAX_PUBLIC_CONNECTIONS,
        &ca,
    );
    let established = blocking(move || {
        let line = load.wait_for("INFLIGHT");
        (load, line)
    })
    .await;
    let (mut load, line) = established;
    assert_eq!(
        line.trim(),
        format!("INFLIGHT {MAX_PUBLIC_CONNECTIONS}"),
        "all {MAX_PUBLIC_CONNECTIONS} exchanges are in flight with their response heads received"
    );

    let (fds_peak, rss_peak, pipes_peak) =
        blocking(|| (open_fds(), rss_kib(), pipe_capacity_octets())).await;
    assert!(
        fds_peak <= fds_before + 768,
        "descriptors stay within the accounted 768: {fds_before} -> {fds_peak}"
    );
    let resident_growth = rss_peak.saturating_sub(rss_before) * 1024;
    let pipe_growth = pipes_peak.saturating_sub(pipes_before);
    assert!(
        resident_growth + pipe_growth <= 24 * 1024 * 1024,
        "resident growth {resident_growth} plus pipe capacity growth {pipe_growth} exceeds the \
         accounted 24 MiB (128 x 192 KiB): the budget is falsified and returns to DESIGN"
    );
    assert!(
        pipe_growth >= 2 * PIPE_CAPACITY_OCTETS,
        "the relay pipes exist while exchanges are in flight: {pipe_growth}"
    );

    // The 129th concurrent connection is closed before any TLS octet.
    let client = world.client();
    blocking(move || client.closed_before_any_tls_octet(FIVE_SECOND_DEADLINE))
        .await
        .expect("the 129th connection is closed before any TLS octet");

    // Drain: every exchange completes its 1 MiB body; the process returns to
    // its baseline.
    let done = blocking(move || {
        load.drain();
        let line = load.wait_for("DONE");
        load.finish();
        line
    })
    .await;
    assert_eq!(done.trim(), format!("DONE {MAX_PUBLIC_CONNECTIONS}"), "every body arrived exact");
    blocking(move || {
        overdrive_system_conformance::wait_until(
            Duration::from_secs(30),
            "descriptors and pipes return to the baseline after drain",
            || open_fds() <= fds_before && pipe_capacity_octets() <= pipes_before,
        );
    })
    .await;

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-105 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-105 (N arm) -- Exact-length and chunked bodies cross the production
/// path octet-exact in both directions at the relay-pipe capacity boundaries,
/// and the next request head is untouched across two keep-alive exchanges.
///
/// Given a serving workload
/// When the user sends `Content-Length` and chunked request bodies of 0, 1,
///      cap-1, cap, cap+1, 2 cap and 2 cap+1 octets (cap = 65 536) and fetches
///      exact-length and chunked response bodies of the same lengths
///  And two requests share one connection
/// Then the guest's digest of each request body equals the digest of what was
///      sent, each response body equals the pattern, and the second exchange on
///      the connection is intact.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_relays_exact_length_and_chunked_bodies_octet_exact_over_keep_alive()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 112, GuestKind::Conformance, ServiceShape::Serving).await;
    let cap = PIPE_CAPACITY_OCTETS as usize;
    let lengths = [0, 1, cap - 1, cap, cap + 1, 2 * cap, 2 * cap + 1];

    let client = world.client();
    blocking(move || {
        for length in lengths {
            let body = pattern_bytes(0, length);
            let digest = format!("len={length} fnv={:016x}", fnv1a(&body));

            // Request side, exact length.
            let echoed = client
                .connect()
                .expect("TLS 1.3 handshake")
                .exchange(&Request::post("/echo", RequestBody::Length(body.clone())))
                .unwrap_or_else(|e| panic!("exact-length echo of {length}: {e:?}"));
            assert_eq!(echoed.status(), 200, "exact-length request of {length}");
            assert_eq!(String::from_utf8_lossy(&echoed.body).trim(), digest, "length {length}");

            // Request side, chunked (three chunks, the middle one split mid-pipe).
            let third = length / 3;
            let chunks: Vec<Vec<u8>> =
                [&body[..third], &body[third..2 * third], &body[2 * third..]]
                    .into_iter()
                    .filter(|piece| !piece.is_empty())
                    .map(<[u8]>::to_vec)
                    .collect();
            let echoed = client
                .connect()
                .expect("TLS 1.3 handshake")
                .exchange(&Request::post("/echo", RequestBody::Chunked(chunks)))
                .unwrap_or_else(|e| panic!("chunked echo of {length}: {e:?}"));
            assert_eq!(echoed.status(), 200, "chunked request of {length}");
            assert_eq!(String::from_utf8_lossy(&echoed.body).trim(), digest, "chunked {length}");

            // Response side, exact length and chunked.
            for (route, chunked) in [("bytes", false), ("chunked", true)] {
                let response = client
                    .connect()
                    .expect("TLS 1.3 handshake")
                    .exchange(&Request::get(&format!("/{route}/{length}")))
                    .unwrap_or_else(|e| panic!("{route} response of {length}: {e:?}"));
                assert_eq!(response.status(), 200);
                assert_eq!(response.body.len(), length, "{route} body length");
                assert_eq!(first_pattern_mismatch(0, &response.body), None, "{route} {length}");
                assert_eq!(!response.chunk_sizes.is_empty() || length == 0, chunked || length == 0);
            }
        }

        // Both AES-GCM suites the gateway offers relay a body octet-exact.
        for suite in [Suite::Aes128, Suite::Aes256] {
            let mut connection = client
                .connect_with(suite, "api.example.com")
                .unwrap_or_else(|e| panic!("{suite:?} handshake: {e}"));
            let expected = match suite {
                Suite::Aes128 => CipherSuite::TLS13_AES_128_GCM_SHA256,
                _ => CipherSuite::TLS13_AES_256_GCM_SHA384,
            };
            assert_eq!(connection.negotiated_suite(), Some(expected), "{suite:?} is negotiated");
            let body = pattern_bytes(0, cap + 1);
            let echoed = connection
                .exchange(&Request::post("/echo", RequestBody::Length(body.clone())))
                .unwrap_or_else(|e| panic!("{suite:?} echo: {e:?}"));
            assert_eq!(
                String::from_utf8_lossy(&echoed.body).trim(),
                format!("len={} fnv={:016x}", body.len(), fnv1a(&body)),
                "{suite:?}"
            );
        }

        // Two keep-alive exchanges on one connection: the second head is intact.
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        let first = connection
            .exchange(&Request::post("/echo", RequestBody::Length(pattern_bytes(0, cap + 1))))
            .expect("first keep-alive exchange");
        assert_eq!(first.status(), 200);
        let second = connection
            .exchange(&Request::get(&format!("/bytes/{cap}")))
            .expect("second keep-alive exchange on the same connection");
        assert_eq!(second.status(), 200);
        assert_eq!(first_pattern_mismatch(0, &second.body), None);
        assert_eq!(second.body.len(), cap);
    })
    .await;

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-109 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-109 (N arm, assumption A-K7) -- A production leg-C sends no KeyUpdate
/// across a response of exactly 16 MiB, the response ceiling. A non-zero
/// `TlsRxRekeyReceived` delta is a deterministic hard failure that falsifies
/// A-K7 and returns to DESIGN; it is neither a pass nor inconclusive.
///
/// Given a serving workload and the kernel TLS counters read
/// When the user fetches a response of exactly 16 MiB
/// Then the body is octet-exact, `TlsRxRekeyReceived` advanced by 0 and
///      `TlsDecryptError` is unchanged.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_sends_no_key_update_across_a_sixteen_mebibyte_response()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 113, GuestKind::Conformance, ServiceShape::Serving).await;

    let before = blocking(TlsStat::read).await;
    let client = world.client();
    let response = blocking(move || {
        client
            .connect()
            .expect("TLS 1.3 handshake")
            .exchange(&Request::get(&format!("/bytes/{BODY_CEILING_OCTETS}")))
            .expect("the 16 MiB response")
    })
    .await;
    let after = blocking(TlsStat::read).await;

    assert_eq!(response.status(), 200, "a response at the ceiling passes");
    assert_eq!(response.body.len() as u64, BODY_CEILING_OCTETS);
    assert_eq!(first_pattern_mismatch(0, &response.body), None, "the body is octet-exact");
    assert_eq!(
        after.delta(&before, "TlsRxRekeyReceived"),
        0,
        "leg-C originated a KeyUpdate across the 16 MiB response: assumption A-K7 is \
         falsified; this returns to DESIGN as a contract-shaping falsification"
    );
    assert_eq!(after.delta(&before, "TlsDecryptError"), 0, "no record failed to decrypt");

    world.tear_down().await;
}

// ---------------------------------------------------------------------------
// S-PIG-115 (N arm)
// ---------------------------------------------------------------------------

/// S-PIG-115 (N arm) -- A client that keeps sending a body after a rejection or
/// an early origin answer reads the whole response and then close_notify, never
/// a reset; the discard ends at the peer's close_notify, at the request-body
/// ceiling, or at its deadline; the typed 408/413/431/505 responses reach a real
/// client.
///
/// Given a serving workload
/// When the client sends a head the gateway rejects (413 on declared length; 400,
///      421, 431 and 505 before any body), or the origin answers 401 early, and
///      the client keeps sending body octets, then sends close_notify
/// Then it reads the typed response, no write or read ends in a reset, and the
///      gateway's side ends with close_notify; a client that never sends
///      close_notify has the connection closed by the gateway within the
///      request deadline; one that sends past the ceiling has it closed there.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER public-request-end-to-end"]
async fn native_metal_kvm_public_ingress_gateway_lets_the_client_read_the_response_before_a_clean_close()
 {
    let trace = TraceHistory::install_global();
    let world =
        GatewayWorld::serving(&trace, 114, GuestKind::Conformance, ServiceShape::Serving).await;
    let oversize_head = format!(
        "GET / HTTP/1.1\r\nHost: api.example.com\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(HEAD_CEILING_OCTETS + 1)
    );

    // Rejections and the early origin answer, then a body that keeps coming and
    // ends with the client's close_notify.
    let rejections: Vec<(&str, Vec<u8>, u16, u64)> = vec![
        (
            "413 on declared length",
            Request::post("/echo", RequestBody::Declared(BODY_CEILING_OCTETS + 1)).head_bytes(),
            413,
            2 * 1024 * 1024,
        ),
        (
            "an early 401 from the origin",
            Request::post("/early", RequestBody::Declared(4 * 1024 * 1024)).head_bytes(),
            401,
            2 * 1024 * 1024,
        ),
        (
            "400 on a non-origin-form target",
            b"CONNECT api.example.com:443 HTTP/1.1\r\nHost: api.example.com\r\n\r\n".to_vec(),
            400,
            256 * 1024,
        ),
        (
            "421 on a Host different from the SNI",
            b"POST /echo HTTP/1.1\r\nHost: other.example.com\r\nContent-Length: 262144\r\n\r\n"
                .to_vec(),
            421,
            256 * 1024,
        ),
        ("431 on a head above the ceiling", oversize_head.into_bytes(), 431, 256 * 1024),
        (
            "505 on an HTTP/1.0 request",
            b"POST /echo HTTP/1.0\r\nHost: api.example.com\r\nContent-Length: 262144\r\n\r\n"
                .to_vec(),
            505,
            256 * 1024,
        ),
    ];
    for (label, head, status, body_octets) in rejections {
        let client = world.client();
        let outcome = blocking(move || client.send_body_after_rejection(&head, body_octets)).await;
        assert_eq!(
            outcome.status,
            Some(status),
            "{label}: the typed response reaches the client: {outcome:?}"
        );
        assert_eq!(outcome.write_failure, None, "{label}: no write was reset: {outcome:?}");
        assert_eq!(
            outcome.end,
            StreamEnd::CloseNotify,
            "{label}: the gateway ends with close_notify, never a reset: {outcome:?}"
        );
    }

    // The discard ends at the request-body ceiling: a client that sends past it
    // has read the response, and the gateway closes at the ceiling.
    let client = world.client();
    let past_ceiling = blocking(move || {
        client.send_body_after_rejection(
            &Request::post("/echo", RequestBody::Declared(BODY_CEILING_OCTETS + 1)).head_bytes(),
            BODY_CEILING_OCTETS + 2 * 1024 * 1024,
        )
    })
    .await;
    assert_eq!(past_ceiling.status, Some(413), "the 413 was read before the discard ended");
    assert_ne!(
        past_ceiling.end,
        StreamEnd::Timeout,
        "the gateway closed at the ceiling: {past_ceiling:?}"
    );

    // The discard ends at its deadline for a client that goes silent and never
    // sends close_notify: the gateway closes within the whole-request deadline.
    let client = world.client();
    let (status, end, waited) = blocking(move || {
        let mut connection = client.connect().expect("TLS 1.3 handshake");
        connection
            .send(
                &Request::post("/echo", RequestBody::Declared(BODY_CEILING_OCTETS + 1))
                    .head_bytes(),
            )
            .expect("send the head");
        connection.set_read_timeout(Some(FIVE_SECOND_DEADLINE));
        let status = connection.read_head().map(|head| head.status).ok();
        let started = Instant::now();
        let (end, _) = connection.read_to_end(Duration::from_secs(120));
        (status, end, started.elapsed())
    })
    .await;
    assert_eq!(status, Some(413));
    assert_ne!(end, StreamEnd::Timeout, "the gateway closes a silent peer at the deadline");
    assert!(waited < Duration::from_secs(120), "inside the whole-request deadline: {waited:?}");

    world.tear_down().await;
}
