//! Production boot composition acceptance tests for
//! `backend-discovery-bridge-service-reachability` (Slice 2 / #175).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-11..S-BDB-17, S-BDB-20.
//!
//! Tier 3 — runs through `cargo xtask lima run -- cargo nextest run
//! -p overdrive-control-plane -E 'test(boot_composition)' --features integration-tests`
//! per `.claude/rules/testing.md` § "Running tests — Lima VM".
//!
//! RED scaffold convention — see `walking_skeleton.rs` module docs.

// ----------------------------------------------------------------------------
// Happy-path boot
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_composes_ebpf_dataplane_and_attaches_xdp_to_both_ifaces() {
    // S-BDB-11 — happy-path boot:
    //   GIVEN valid [dataplane] config pointing at lb_veth_a / lb_veth_b on Lima
    //   WHEN  serve_with_config runs
    //   THEN  bpftool prog show reveals xdp_service_map_lookup attached to lb_veth_a
    //         AND xdp_reverse_nat_lookup attached to lb_veth_b
    //         AND /sys/fs/bpf/overdrive/SERVICE_MAP pin exists
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-11 / boot composes EbpfDataplane: \
         XDP programs attached to both client_iface and backend_iface, \
         SERVICE_MAP bpffs pin created)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_resolves_host_ipv4_via_getifaddrs_on_client_iface() {
    // S-BDB-16 — D4 happy path:
    //   GIVEN lb_veth_a configured with known IPv4 (e.g., 10.42.0.1)
    //   WHEN  serve_with_config runs
    //   THEN  resolve_iface_ipv4("lb_veth_a") returns Ok(10.42.0.1)
    //         AND AppState.host_ipv4 == 10.42.0.1
    //         AND a subsequent Service submission results in BACKEND_MAP entries
    //             with ipv4 == 10.42.0.1 (subsumed by walking-skeleton assertion)
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-16 / D4 happy path: \
         resolve_iface_ipv4 returns configured iface's IPv4, AppState carries it)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_succeeds_when_earned_trust_probe_round_trips_backend_map() {
    // S-BDB-15 — D2 happy path:
    //   GIVEN valid [dataplane] config on Lima
    //   WHEN  serve_with_config runs
    //   THEN  EbpfDataplane::new succeeds
    //         AND EbpfDataplane::probe returns Ok(())
    //         AND BACKEND_MAP::get(BackendId::PROBE = u32::MAX, cpu = 0) returns None
    //             after probe completion (sentinel deleted, no leak)
    //         AND boot proceeds past the probe call site
    //         AND server reaches listener-bind (TLS handshake observable)
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-15 / D2 happy path: \
         Earned-Trust probe writes+reads+deletes sentinel BACKEND_MAP entry, \
         boot proceeds past probe call site)"
    );
}

// ----------------------------------------------------------------------------
// Error-path boot
// ----------------------------------------------------------------------------

#[test]
fn boot_refuses_when_dataplane_config_section_missing() {
    // S-BDB-12 — missing config section closure (step 02-01):
    //   GIVEN overdrive.toml with no [dataplane] section
    //   WHEN  parse_dataplane_section runs against the operator-supplied
    //         TOML
    //   THEN  result is ControlPlaneError::Validation { message: "missing required
    //         [dataplane] section in overdrive.toml (client_iface + backend_iface)",
    //         field: Some("dataplane") }
    //
    // Per architecture.md § 5.1 + step task. The full
    // `run_server_with_obs_and_driver` boot refusal (the original
    // RED scaffold scope) is exercised by the inline unit test
    // `dataplane_config::tests::boot_refuses_when_dataplane_section_missing`
    // which pins the parser-level contract. The boot path threads
    // this through `config.dataplane.as_ref().ok_or_else(...)`
    // returning the same Validation shape; integration-level
    // exercise lands in step 02-02 alongside EbpfDataplane wiring.
    use overdrive_control_plane::dataplane_config::parse_dataplane_section;
    use overdrive_control_plane::error::ControlPlaneError;

    let result = parse_dataplane_section("");
    match result {
        Err(ControlPlaneError::Validation { message, field }) => {
            assert_eq!(field.as_deref(), Some("dataplane"));
            assert!(
                message.contains("missing required [dataplane] section"),
                "expected verbatim 'missing required [dataplane] section', got: {message}",
            );
            assert!(
                message.contains("client_iface") && message.contains("backend_iface"),
                "expected message to name both required keys, got: {message}",
            );
        }
        other => panic!("expected Validation on missing section, got {other:?}"),
    }
}

#[test]
fn dataplane_boot_error_iface_addr_resolution_display_contains_remediation() {
    // Step 02-01 unit test (d): the IfaceAddrResolution variant's
    // Display form MUST embed the iface name AND the operator-
    // actionable `ip -4 addr show` remediation hint per
    // architecture.md § 5.3 verbatim. The structural defense
    // against rewording: the assertion names both load-bearing
    // tokens so a future refactor that strips either flips the
    // test to red.
    use overdrive_control_plane::error::DataplaneBootError;

    let err = DataplaneBootError::IfaceAddrResolution {
        iface: "lb_veth_ipv6only".to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "no IPv4"),
    };
    let display = format!("{err}");
    assert!(
        display.contains("lb_veth_ipv6only"),
        "expected iface name 'lb_veth_ipv6only' in Display, got: {display}",
    );
    assert!(
        display.contains("ip -4 addr show"),
        "expected remediation 'ip -4 addr show' in Display, got: {display}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_refuses_when_client_iface_does_not_exist() {
    // S-BDB-13 — D4 / Q175.1 invalid-iface error path:
    //   GIVEN [dataplane] client_iface = "definitely-not-an-iface-foo"
    //   WHEN  serve_with_config runs
    //   THEN  error is ControlPlaneError::DataplaneBoot(DataplaneBootError::Construct {
    //             client_iface: "definitely-not-an-iface-foo", backend_iface: "lb_veth_b",
    //             source: DataplaneError::IfaceNotFound { .. } })
    //         AND Display form names the iface AND suggests `ip link show <iface>`
    //         AND no XDP attached to backend_iface (construction aborts pre-attach)
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-13 / D4 invalid client_iface: \
         boot refuses with DataplaneBoot(Construct {{ source: IfaceNotFound }}), \
         operator-actionable Display, no partial XDP attach)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_refuses_when_earned_trust_probe_fails() {
    // S-BDB-14 — D2 probe failure:
    //   GIVEN BACKEND_MAP programmability intentionally degraded
    //         (test fixture pre-populates sentinel BackendId or injects probe fault)
    //   WHEN  serve_with_config runs
    //   THEN  EbpfDataplane::new succeeds (load + attach OK)
    //         AND EbpfDataplane::probe returns Err(DataplaneError::LoadFailed(...))
    //             with substring "probe: round-trip mismatch" or "probe: BACKEND_MAP"
    //         AND error is ControlPlaneError::DataplaneBoot(DataplaneBootError::Probe {
    //             source: DataplaneError::LoadFailed(_) })
    //         AND structured `health.startup.refused` event with reason = "dataplane.probe"
    //         AND test fixture cleans up partial XDP attach + bpffs pin
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-14 / D2 probe failure: \
         boot refuses with DataplaneBoot(Probe), health.startup.refused emitted, \
         partial state cleaned up)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn boot_refuses_when_getifaddrs_resolution_fails_for_iface() {
    // S-BDB-17 — D4 getifaddrs failure:
    //   GIVEN veth pair lb_veth_ipv6only configured WITHOUT IPv4 address
    //         (iface exists per `ip link show` but no `inet` entry in `ip -4 addr show`)
    //         AND [dataplane] client_iface = "lb_veth_ipv6only"
    //   WHEN  serve_with_config runs
    //   THEN  resolve_iface_ipv4("lb_veth_ipv6only") returns Err(io::Error)
    //             with NotFound or Other (the getifaddrs no-IPv4 case)
    //         AND error is ControlPlaneError::DataplaneBoot(DataplaneBootError
    //             ::IfaceAddrResolution { iface: "lb_veth_ipv6only", source })
    //         AND Display form names the iface AND suggests `ip -4 addr show <iface>`
    //         AND partial XDP attach (if any) cleaned up on Drop
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-17 / D4 getifaddrs failure: \
         boot refuses with DataplaneBoot(IfaceAddrResolution), \
         operator-actionable Display, Drop cleans up partial state)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn attach_mode_fallback_emits_xdp_attach_fallback_generic_event() {
    // S-BDB-20 — Q175.3 attach-mode fallback:
    //   GIVEN dummy0 interface created via `ip link add dummy0 type dummy`
    //         (dummy driver does NOT implement native XDP)
    //         AND [dataplane] client_iface = "dummy0"
    //         AND tracing subscriber installed by test fixture
    //   WHEN  serve_with_config runs and EbpfDataplane::new attempts attach on dummy0
    //   THEN  exactly one structured event with name "xdp.attach.fallback_generic"
    //             with fields iface = "dummy0" AND errno = EOPNOTSUPP (or ENOTSUP)
    //         AND SKB_MODE retry succeeds
    //         AND ip link show dummy0 reveals xdpgeneric attachment (not xdpdrv)
    //         AND EbpfDataplane::new returns Ok(_)
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-20 / Q175.3 attach-mode fallback: \
         dummy iface forces native rejection -> structured fallback event emitted \
         once -> SKB_MODE retry succeeds -> xdpgeneric attached)"
    );
}
