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
//! Closed in step 02-02 of
//! `backend-discovery-bridge-service-reachability/deliver`:
//! - S-BDB-11 (boot composes EbpfDataplane: XDP attached to both ifaces)
//! - S-BDB-13 (D4 / Q175.1 invalid client_iface refusal)
//! - S-BDB-16 (D4 happy path: host_ipv4 resolved via getifaddrs)
//! - S-BDB-17 (D4 getifaddrs no-IPv4 refusal)
//! - S-BDB-18 (graceful shutdown — XDP detach + bpffs pin removed)
//! - S-BDB-20 (Q175.3 attach-mode fallback on dummy iface)
//!
//! The S-BDB-12 (missing-section refusal) closure landed in step 02-01.
//! The Earned-Trust probe tests (S-BDB-14, S-BDB-15) close in step 02-03.
//!
//! ## Tier 3 cleanup discipline
//!
//! Per `.claude/rules/debugging.md` § "Leftover XDP attachments across runs",
//! every test that calls `EbpfDataplane::new_with_pin_dir` (directly or via
//! `run_server`) MUST clean up XDP attachments + bpffs pins on its way out —
//! including assertion-failure paths. The [`BootFixture`] RAII guard below
//! owns the teardown: it (a) detaches XDP from each test-managed iface via
//! `ip link set ... xdp* off`, (b) unlinks the per-test bpffs pin file, (c)
//! deletes test-created veth pairs and dummy interfaces. Drop is best-effort
//! — failures are logged at debug — because by the time Drop runs we may be
//! unwinding from an assertion panic.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines
)]

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use overdrive_control_plane::{ServerConfig, ServerHandle, dataplane_config::DataplaneConfig};
use overdrive_host::RealCgroupFs;
use tempfile::TempDir;

/// Builds the standard test-time `Arc<dyn CgroupFs>` adapter — real
/// `tokio::fs::*` against `/sys/fs/cgroup` per step 01-06 of the
/// `cgroup-fs-port` migration. Wraps a `RealCgroupFs::new()` in `Arc`
/// at the call site so each test that boots a server has its own
/// `Arc<dyn CgroupFs>` instance.
fn test_cgroup_fs() -> std::sync::Arc<dyn overdrive_core::traits::cgroup_fs::CgroupFs> {
    std::sync::Arc::new(RealCgroupFs::new())
}

// ----------------------------------------------------------------------------
// Test-managed interface fixture (RAII)
// ----------------------------------------------------------------------------

/// Bump-allocated counter so adjacent tests do not collide on iface names
/// even when run in parallel. nextest's default thread pool can interleave
/// these tests inside a single binary; the per-test name keeps the `ip(8)`
/// state separate.
static IFACE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Allocate a fresh per-test iface-name pair. Mixes the PID into the
/// suffix so nextest's `tests::run` (which spawns one process per test
/// binary OR fork-per-test under leak detection) does not produce two
/// tests with counter=0 racing on the same iface name. Linux iface
/// names are bounded to 15 bytes — keep the suffix short.
fn next_iface_names() -> (String, String) {
    let n = IFACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    // `std::process::id()` returns u32; truncate to 4 hex digits so the
    // resulting `ovd-{pid}-{n}{a|b}` name stays under 15 bytes for
    // small `n` values.
    let pid = std::process::id() & 0xFFFF;
    (format!("ovd-{pid:04x}-{n}a"), format!("ovd-{pid:04x}-{n}b"))
}

/// Spawn `ip(8)` with the given args. `Result::Ok(())` on success;
/// returns the captured stderr on non-zero exit so callers can decide
/// whether to fail the test (setup) or swallow (teardown).
fn run_ip(args: &[&str]) -> Result<(), String> {
    let out =
        Command::new("ip").args(args).output().map_err(|e| format!("spawn ip {args:?}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ip {args:?} exit={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim(),
        ))
    }
}

/// Best-effort `ip(8)` — swallows errors. Used in cleanup paths where
/// a missing interface is a no-op success.
fn run_ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).output();
}

/// Read `ip link show <iface>` once. Returns the captured stdout (lossy
/// UTF-8) so callers can `.contains("xdpgeneric")` / `.contains("xdpdrv")`.
fn ip_link_show(iface: &str) -> Result<String, String> {
    let out = Command::new("ip")
        .args(["link", "show", iface])
        .output()
        .map_err(|e| format!("spawn ip link show {iface}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ip link show {iface} exit={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Check whether `ip link show <iface>` reports any XDP attachment.
/// Recognises native (`xdpdrv`), generic (`xdpgeneric`), or the
/// bare `xdp ` marker iproute2 prints on some kernels.
fn iface_has_xdp(iface: &str) -> Result<bool, String> {
    let out = ip_link_show(iface)?;
    Ok(out.contains("xdpdrv") || out.contains("xdpgeneric") || out.contains("xdp "))
}

/// Per-test fixture: builds veth pair + bpffs pin directory; tears them
/// down (and any XDP attached to them, and any pin left in the bpffs
/// dir) on Drop.
struct BootFixture {
    client_iface: String,
    backend_iface: String,
    pin_dir: PathBuf,
    _bpffs_root: TempDir,
}

impl BootFixture {
    /// Set up a fresh veth pair (`<client>`, `<backend>`), bring both
    /// up, assign `client_cidr` to the client side so getifaddrs has
    /// an IPv4 to find, and create a per-test bpffs pin directory.
    ///
    /// The bpffs dir lives under `/sys/fs/bpf/overdrive-test-<rand>` —
    /// a per-test subdirectory of bpffs so concurrent tests do not
    /// collide on the SERVICE_MAP pin path.
    fn setup_veth_pair(client_cidr: Option<&str>) -> Result<Self, String> {
        let (client, backend) = next_iface_names();
        // Pre-clean any leftover state from a prior aborted run.
        run_ip_quiet(&["link", "del", &client]);
        run_ip_quiet(&["link", "del", &backend]);

        run_ip(&["link", "add", &client, "type", "veth", "peer", "name", &backend])?;
        run_ip(&["link", "set", &client, "up"])?;
        run_ip(&["link", "set", &backend, "up"])?;
        if let Some(cidr) = client_cidr {
            run_ip(&["addr", "add", cidr, "dev", &client])?;
        }

        let bpffs_root = tempfile::Builder::new()
            .prefix("overdrive-test-")
            .tempdir_in("/sys/fs/bpf")
            .map_err(|e| format!("tempdir under /sys/fs/bpf: {e}"))?;
        let pin_dir = bpffs_root.path().to_path_buf();

        Ok(Self { client_iface: client, backend_iface: backend, pin_dir, _bpffs_root: bpffs_root })
    }

    /// Set up a single dummy interface (no veth peer) with an explicit
    /// `xdpdrv` rejection so the EbpfDataplane attach path exercises
    /// the fallback-to-SKB code. Used by S-BDB-20.
    fn setup_dummy() -> Result<Self, String> {
        let (client, backend) = next_iface_names();
        run_ip_quiet(&["link", "del", &client]);
        run_ip_quiet(&["link", "del", &backend]);
        // Two dummy ifaces — EbpfDataplane requires both client + backend
        // ifaces resolvable. dummy driver does NOT implement native XDP,
        // so attach falls back to xdpgeneric.
        run_ip(&["link", "add", &client, "type", "dummy"])?;
        run_ip(&["link", "add", &backend, "type", "dummy"])?;
        run_ip(&["link", "set", &client, "up"])?;
        run_ip(&["link", "set", &backend, "up"])?;
        // Assign IPv4 to client so getifaddrs succeeds (the boot path
        // resolves host_ipv4 from client_iface).
        run_ip(&["addr", "add", "10.244.200.1/24", "dev", &client])?;

        let bpffs_root = tempfile::Builder::new()
            .prefix("overdrive-test-")
            .tempdir_in("/sys/fs/bpf")
            .map_err(|e| format!("tempdir under /sys/fs/bpf: {e}"))?;
        let pin_dir = bpffs_root.path().to_path_buf();

        Ok(Self { client_iface: client, backend_iface: backend, pin_dir, _bpffs_root: bpffs_root })
    }

    fn dataplane_config(&self) -> DataplaneConfig {
        DataplaneConfig {
            client_iface: self.client_iface.clone(),
            backend_iface: self.backend_iface.clone(),
        }
    }
}

impl Drop for BootFixture {
    fn drop(&mut self) {
        // Detach any XDP attached to either iface — best-effort, in
        // every attach mode.
        for mode in ["xdpgeneric", "xdpdrv", "xdp"] {
            run_ip_quiet(&["link", "set", "dev", &self.client_iface, mode, "off"]);
            run_ip_quiet(&["link", "set", "dev", &self.backend_iface, mode, "off"]);
        }
        // Unlink the SERVICE_MAP pin if EbpfDataplane left it behind
        // (Drop normally handles this, but a panic in `new_with_pin_dir`
        // before construction completes leaves the pin in place).
        let pin_path = self.pin_dir.join("SERVICE_MAP");
        let _ = std::fs::remove_file(&pin_path);
        // Delete the ifaces — kernel auto-removes the peer when one
        // side is destroyed.
        run_ip_quiet(&["link", "del", &self.client_iface]);
        run_ip_quiet(&["link", "del", &self.backend_iface]);
        // `_bpffs_root: TempDir` drops here and removes the tempdir
        // under /sys/fs/bpf.
    }
}

/// Build a `ServerConfig` pointed at the fixture's veth/dummy ifaces
/// and per-test bpffs pin dir. `data_dir` and `operator_config_dir`
/// are per-test tempdirs (host filesystem, not bpffs).
fn server_config_with(
    fx: &BootFixture,
    data_dir: &Path,
    operator_config_dir: &Path,
) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind"),
        data_dir: data_dir.to_path_buf(),
        operator_config_dir: operator_config_dir.to_path_buf(),
        dataplane: Some(fx.dataplane_config()),
        dataplane_pin_dir: Some(fx.pin_dir.clone()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    }
}

/// Skip the test (with a clear message) when `ip(8)` rejects veth
/// creation for capability reasons. Tier 3 tests run inside Lima as
/// root via `cargo xtask lima run --` per `.claude/rules/testing.md`
/// § "Running tests — Lima VM"; outside Lima the test environment
/// lacks CAP_NET_ADMIN and the test cannot exercise the production
/// path.
fn require_cap_net_admin() {
    // Quick probe: ask the kernel for our effective capability set
    // indirectly by trying a no-op `ip link add` (we delete it
    // immediately). If EPERM/EACCES, eprintln + skip.
    let probe = "ovd-bdb-capprobe";
    run_ip_quiet(&["link", "del", probe]);
    let result = Command::new("ip").args(["link", "add", probe, "type", "dummy"]).output();
    let ok = match result {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            !(stderr.contains("Operation not permitted") || stderr.contains("Permission denied"))
        }
        Err(_) => false,
    };
    run_ip_quiet(&["link", "del", probe]);
    if !ok {
        eprintln!("skipping: CAP_NET_ADMIN required; run via `cargo xtask lima run --`");
        // Abort the test thread without failing the suite.
        panic!("BOOT_COMPOSITION_SKIPPED_NO_CAP_NET_ADMIN");
    }
}

// ----------------------------------------------------------------------------
// Happy-path boot
// ----------------------------------------------------------------------------

/// S-BDB-11 — happy-path boot exercises the full production
/// `EbpfDataplane` composition through `run_server` and asserts on
/// kernel-observable side effects:
///   - both ifaces report an XDP attachment after boot
///   - `<pin_dir>/SERVICE_MAP` exists
///
/// Subsumes the in-process bridge-to-hydrator handoff (S-BDB-19), the
/// in-process service-map hydrator dispatch, and the walking-skeleton
/// TCP-round-trip (S-BDB-01) — those are tested separately by their
/// owning steps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_composes_ebpf_dataplane_and_attaches_xdp_to_both_ifaces() {
    require_cap_net_admin();

    let fx = BootFixture::setup_veth_pair(Some("10.244.0.1/24")).expect("veth pair setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);
    let pin_path = fx.pin_dir.join("SERVICE_MAP");

    let handle: ServerHandle =
        overdrive_control_plane::run_server(config, test_cgroup_fs()).await.expect("run_server");

    // Both ifaces must carry an XDP attachment.
    assert!(
        iface_has_xdp(&fx.client_iface).expect("inspect client iface"),
        "expected XDP on client_iface {}; ip link output: {}",
        fx.client_iface,
        ip_link_show(&fx.client_iface).unwrap_or_default(),
    );
    assert!(
        iface_has_xdp(&fx.backend_iface).expect("inspect backend iface"),
        "expected XDP on backend_iface {}; ip link output: {}",
        fx.backend_iface,
        ip_link_show(&fx.backend_iface).unwrap_or_default(),
    );

    // SERVICE_MAP pin exists.
    assert!(pin_path.exists(), "expected SERVICE_MAP pin at {}", pin_path.display());

    // Shutdown — drops EbpfDataplane (XDP detaches, pin unlinks).
    handle.shutdown(std::time::Duration::from_secs(2)).await;
}

/// S-BDB-16 — D4 happy path: `resolve_iface_ipv4` derives
/// `AppState.host_ipv4` at boot. We assert by observable behaviour:
/// the boot succeeds when client_iface has an IPv4 (the contract per
/// architecture.md § 5.1 says missing-IPv4 refuses boot — see S-BDB-17),
/// so a successful boot here proves the resolution path returned Ok.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_resolves_host_ipv4_via_getifaddrs_on_client_iface() {
    require_cap_net_admin();

    // Use a deterministic CIDR so the test can assert getifaddrs
    // would return this IP. The boot path's `resolve_iface_ipv4` call
    // is internal; the observable contract is "boot succeeds with this
    // IPv4 configured" + "boot refuses with no IPv4 configured"
    // (S-BDB-17).
    let cidr = "10.244.42.7/24";
    let fx = BootFixture::setup_veth_pair(Some(cidr)).expect("veth setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);

    // Sanity-check that getifaddrs (the production path) resolves the
    // expected address. This narrows a boot failure between "the
    // resolution code is wrong" and "the test setup didn't configure
    // the address" without instrumenting production.
    let expected: Ipv4Addr = "10.244.42.7".parse().expect("parse expected");
    let resolved = overdrive_control_plane::iface::resolve_iface_ipv4(&fx.client_iface)
        .expect("resolve_iface_ipv4 must succeed when client_iface has IPv4");
    assert_eq!(
        resolved, expected,
        "expected getifaddrs to return {expected} for client_iface {}",
        fx.client_iface,
    );

    // Production-path: boot must succeed end-to-end with this iface.
    let handle: ServerHandle = overdrive_control_plane::run_server(config, test_cgroup_fs())
        .await
        .expect("run_server with IPv4-bearing client_iface");
    handle.shutdown(std::time::Duration::from_secs(2)).await;
}

// ----------------------------------------------------------------------------
// Graceful shutdown
// ----------------------------------------------------------------------------

/// S-BDB-18 — graceful shutdown via the `EbpfDataplane` Drop impl
/// detaches XDP from both ifaces and removes the SERVICE_MAP bpffs pin.
///
/// Exercises the Drop RAII contract added to `EbpfDataplane` in this
/// step. The companion test in `walking_skeleton.rs` (same name)
/// exercises the same property at the walking-skeleton scope; this
/// variant lives in the boot-composition module so the cleanup
/// guarantee is asserted by a test that owns the boot fixture.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn graceful_shutdown_detaches_xdp_and_removes_bpffs_pin() {
    require_cap_net_admin();

    let fx = BootFixture::setup_veth_pair(Some("10.244.18.1/24")).expect("veth setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);
    let pin_path = fx.pin_dir.join("SERVICE_MAP");
    let client_name = fx.client_iface.clone();
    let backend_name = fx.backend_iface.clone();

    let handle: ServerHandle =
        overdrive_control_plane::run_server(config, test_cgroup_fs()).await.expect("run_server");

    // Pre-condition: both ifaces have XDP + pin exists. If this fails
    // we are testing the wrong property (the happy path didn't run);
    // see S-BDB-11 for the canonical happy-path assertion.
    assert!(iface_has_xdp(&client_name).expect("inspect client"));
    assert!(iface_has_xdp(&backend_name).expect("inspect backend"));
    assert!(pin_path.exists(), "pin must exist before shutdown");

    // Graceful shutdown. ServerHandle::shutdown drops the EbpfDataplane
    // held by the runtime, which fires Drop → XDP detach + pin unlink.
    handle.shutdown(std::time::Duration::from_secs(2)).await;

    // Post-condition: neither iface carries an XDP attachment and the
    // bpffs pin is gone. aya's `XdpLinkId::Drop` handles the detach;
    // EbpfDataplane::Drop unlinks the pin.
    assert!(
        !iface_has_xdp(&client_name).expect("post-shutdown client inspect"),
        "expected XDP detached from {client_name} after shutdown; ip link: {}",
        ip_link_show(&client_name).unwrap_or_default(),
    );
    assert!(
        !iface_has_xdp(&backend_name).expect("post-shutdown backend inspect"),
        "expected XDP detached from {backend_name} after shutdown; ip link: {}",
        ip_link_show(&backend_name).unwrap_or_default(),
    );
    assert!(
        !pin_path.exists(),
        "expected SERVICE_MAP pin at {} to be removed after shutdown",
        pin_path.display(),
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

/// S-BDB-13 — D4 / Q175.1 invalid-iface refusal:
///   GIVEN client_iface = "definitely-not-an-iface-foo"
///   WHEN  run_server runs
///   THEN  error is ControlPlaneError::DataplaneBoot(
///             DataplaneBootError::IfaceAddrResolution { iface, .. })
///         (the resolve_iface_ipv4 step fires BEFORE EbpfDataplane::new
///          so the typed variant surfaces from that earlier gate;
///          either typed variant satisfies the "refuses to boot with
///          a typed DataplaneBoot error naming the iface" contract).
///         AND no XDP attached to the (non-existent) backend_iface
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_refuses_when_client_iface_does_not_exist() {
    // No CAP_NET_ADMIN gate here — we never create real ifaces. The
    // boot path resolves the iface name to an ifindex (or to its
    // IPv4 via getifaddrs) before any priviledged kernel call; both
    // surface a typed error variant naming the iface.
    let nonexistent_client = "ovd-bdb-nonexistent-foo".to_owned();
    let bpffs_root = tempfile::Builder::new()
        .prefix("overdrive-test-")
        .tempdir_in("/sys/fs/bpf")
        .expect("tempdir under /sys/fs/bpf");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind"),
        data_dir,
        operator_config_dir: cfg_dir,
        dataplane: Some(DataplaneConfig {
            client_iface: nonexistent_client.clone(),
            backend_iface: nonexistent_client.clone(),
        }),
        dataplane_pin_dir: Some(bpffs_root.path().to_path_buf()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let result = overdrive_control_plane::run_server(config, test_cgroup_fs()).await;

    use overdrive_control_plane::error::{ControlPlaneError, DataplaneBootError};
    let err = result.expect_err("expected boot refusal for nonexistent iface");
    let display = format!("{err}");
    // The error chain must name the iface AND surface as the typed
    // DataplaneBoot family — either the early IfaceAddrResolution
    // (resolve_iface_ipv4 path) or the later Construct (EbpfDataplane
    // ifindex resolution path). Both signal "iface invalid", both
    // carry the iface name in Display.
    let is_typed_dataplane = matches!(
        &err,
        ControlPlaneError::DataplaneBoot(
            DataplaneBootError::IfaceAddrResolution { iface, .. }
                | DataplaneBootError::Construct { client_iface: iface, .. }
        ) if iface == &nonexistent_client
    );
    assert!(
        is_typed_dataplane,
        "expected ControlPlaneError::DataplaneBoot(IfaceAddrResolution|Construct) \
         naming {nonexistent_client}, got: {err:?} (display: {display})",
    );
    assert!(
        display.contains(&nonexistent_client),
        "expected iface name in Display, got: {display}",
    );

    // Best-effort cleanup of the pin dir (the boot bailed before
    // pinning, but the tempdir under /sys/fs/bpf still needs removal —
    // owned by `bpffs_root: TempDir`, drops here).
    drop(bpffs_root);
}

/// S-BDB-17 — D4 getifaddrs failure: an iface that exists but carries
/// no IPv4 binding refuses boot with the typed `IfaceAddrResolution`
/// variant. Display names the iface and the `ip -4 addr show <iface>`
/// remediation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_refuses_when_iface_has_no_ipv4_address() {
    require_cap_net_admin();

    // Set up veth pair WITHOUT assigning IPv4 — the iface exists per
    // `ip link show`, but `ip -4 addr show` returns no `inet` entry.
    // getifaddrs returns NotFound; the boot path maps that to
    // DataplaneBootError::IfaceAddrResolution.
    let fx = BootFixture::setup_veth_pair(None).expect("veth setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);
    let client_name = fx.client_iface.clone();

    let result = overdrive_control_plane::run_server(config, test_cgroup_fs()).await;

    use overdrive_control_plane::error::{ControlPlaneError, DataplaneBootError};
    let err = result.expect_err("expected boot refusal for IPv4-less iface");
    match &err {
        ControlPlaneError::DataplaneBoot(DataplaneBootError::IfaceAddrResolution {
            iface, ..
        }) => {
            assert_eq!(
                iface, &client_name,
                "expected IfaceAddrResolution.iface == {client_name}, got {iface}",
            );
        }
        other => panic!(
            "expected DataplaneBoot(IfaceAddrResolution) for IPv4-less iface, got: {other:?}"
        ),
    }
    // Display form names iface + remediation.
    let display = format!("{err}");
    assert!(
        display.contains(&client_name),
        "expected iface name {client_name} in Display, got: {display}",
    );
    assert!(
        display.contains("ip -4 addr show"),
        "expected `ip -4 addr show` remediation in Display, got: {display}",
    );
}

// ----------------------------------------------------------------------------
// S-BDB-20 — attach-mode fallback on dummy iface
// ----------------------------------------------------------------------------

/// Captures `tracing::warn!` events whose target ends with the
/// supplied name. Used to assert S-BDB-20's
/// `xdp.attach.fallback_generic` structured event fires.
///
/// Subscriber is process-global; install once per test via
/// `tracing::subscriber::with_default` so the install only spans
/// the test body.
mod event_capture {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::registry::LookupSpan;

    #[derive(Default)]
    pub struct CapturedFields {
        pub iface: Option<String>,
        pub other: std::collections::BTreeMap<String, String>,
    }

    impl Visit for CapturedFields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            let val = format!("{value:?}");
            if field.name() == "iface" {
                // Strip leading/trailing quotes the Debug formatter adds
                // for &str.
                let trimmed = val.trim_matches('"').to_owned();
                self.iface = Some(trimmed);
            } else if field.name() == "reason" {
                let trimmed = val.trim_matches('"').to_owned();
                self.other.insert("reason".to_owned(), trimmed);
            } else {
                self.other.insert(field.name().to_owned(), val);
            }
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "iface" {
                self.iface = Some(value.to_owned());
            } else {
                self.other.insert(field.name().to_owned(), value.to_owned());
            }
        }
    }

    /// One captured event row.
    #[derive(Debug, Clone)]
    pub struct EventRow {
        pub name: String,
        pub iface: Option<String>,
        pub fields: std::collections::BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    pub struct EventCollector {
        inner: Arc<Mutex<Vec<EventRow>>>,
    }

    impl EventCollector {
        pub fn snapshot(&self) -> Vec<EventRow> {
            self.inner.lock().expect("collector lock").clone()
        }
    }

    impl<S> Layer<S> for EventCollector
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = CapturedFields::default();
            event.record(&mut fields);
            // The event `name` is the `tracing::warn!(name: "...", ...)`
            // first-positional argument. tracing exposes this via
            // `Metadata::name()`.
            let metadata = event.metadata();
            let mut fields_map: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for (k, v) in &fields.other {
                fields_map.insert(k.clone(), v.clone());
            }
            self.inner.lock().expect("collector lock").push(EventRow {
                name: metadata.name().to_owned(),
                iface: fields.iface,
                fields: fields_map,
            });
        }
    }
}

/// S-BDB-20 — Q175.3 attach-mode fallback on dummy iface.
///
/// `dummy` driver does NOT implement native XDP — `xdpdrv` attach
/// returns `EOPNOTSUPP`/`ENOTSUP`, and the production loader
/// (`crates/overdrive-dataplane/src/lib.rs` →
/// `should_fallback_to_generic`) emits a single structured
/// `xdp.attach.fallback_generic` event per iface and retries with
/// `SKB_MODE`. We assert:
///   - boot succeeds (the fallback actually retries successfully)
///   - at least one `xdp.attach.fallback_generic` event names a
///     test-managed iface
///   - the post-boot iface reports `xdpgeneric` attachment
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn attach_mode_fallback_emits_structured_event_on_dummy_iface() {
    use tracing_subscriber::layer::SubscriberExt as _;

    require_cap_net_admin();

    let fx = BootFixture::setup_dummy().expect("dummy iface setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);

    let collector = event_capture::EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());

    // Install the subscriber as the process-wide default for the
    // duration of this boot (tracing's subscriber registration is
    // process-global; the `#[serial(env)]` guard serialises with
    // other tests that install a subscriber).
    let _guard = tracing::subscriber::set_default(subscriber);

    let client_name = fx.client_iface.clone();
    let backend_name = fx.backend_iface.clone();
    let boot_result = overdrive_control_plane::run_server(config, test_cgroup_fs()).await;

    // Boot must succeed — fallback should have retried successfully.
    let handle = boot_result.expect("expected boot to succeed via SKB fallback on dummy iface");

    // Inspect the captured events. The fallback emit shape is
    // `tracing::warn!(name: "xdp.attach.fallback_generic", iface = %iface, ...)`.
    // tracing surfaces the `name:` argument via `Metadata::name()`.
    let events = collector.snapshot();
    let fallback_events: Vec<_> =
        events.iter().filter(|e| e.name == "xdp.attach.fallback_generic").collect();
    assert!(
        !fallback_events.is_empty(),
        "expected at least one xdp.attach.fallback_generic event; saw {} events: {:?}",
        events.len(),
        events.iter().map(|e| &e.name).collect::<Vec<_>>(),
    );
    let names_a_test_iface = fallback_events.iter().any(
        |e| matches!(&e.iface, Some(iface) if iface == &client_name || iface == &backend_name),
    );
    assert!(
        names_a_test_iface,
        "expected fallback event to name {client_name} or {backend_name}, got: {fallback_events:?}",
    );

    // Post-condition: at least one of the ifaces reports xdpgeneric
    // (not xdpdrv) — the fallback actually landed on SKB mode.
    let client_link = ip_link_show(&client_name).unwrap_or_default();
    let backend_link = ip_link_show(&backend_name).unwrap_or_default();
    let saw_generic = client_link.contains("xdpgeneric") || backend_link.contains("xdpgeneric");
    assert!(
        saw_generic,
        "expected xdpgeneric attachment after fallback; client: {client_link} backend: {backend_link}",
    );

    handle.shutdown(std::time::Duration::from_secs(2)).await;
}

// ----------------------------------------------------------------------------
// S-BDB-14 / S-BDB-15 — Earned-Trust probe
// ----------------------------------------------------------------------------

/// S-BDB-14 — production boot refuses when the Earned-Trust probe
/// fails.
///
/// `EbpfDataplane::new` succeeds (load + attach OK), then
/// `EbpfDataplane::probe` returns `Err(DataplaneError::LoadFailed(...))`
/// — we drive this via the `#[cfg(any(test, feature = "integration-tests"))]`
/// `dataplane_probe_fault` injection seam on `ServerConfig`. The boot
/// path maps the probe error to
/// `ControlPlaneError::DataplaneBoot(DataplaneBootError::Probe { source })`
/// and emits a structured `health.startup.refused` event with
/// `reason = "dataplane.probe"` per architecture.md § 5.4.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_refuses_when_earned_trust_probe_fails() {
    use tracing_subscriber::layer::SubscriberExt as _;

    require_cap_net_admin();

    let fx = BootFixture::setup_veth_pair(Some("10.244.14.1/24")).expect("veth setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");

    // Inject the probe-fault message. The verbatim text tracks the
    // architecture.md § 5.4 contract — the boot path reconstructs
    // `DataplaneError::LoadFailed(msg)` from this string and the
    // error chain surfaces "probe: round-trip mismatch" OR
    // "probe: BACKEND_MAP" through `Display`. See the
    // `ServerConfig::dataplane_probe_fault` docstring for why the
    // seam is `String`-shaped at this boundary.
    let fault_msg = "probe: round-trip mismatch (injected by S-BDB-14 fixture)".to_owned();
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind"),
        data_dir,
        operator_config_dir: cfg_dir,
        dataplane: Some(fx.dataplane_config()),
        dataplane_pin_dir: Some(fx.pin_dir.clone()),
        dataplane_probe_fault: Some(fault_msg),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let pin_path = fx.pin_dir.join("SERVICE_MAP");

    let collector = event_capture::EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let result = overdrive_control_plane::run_server(config, test_cgroup_fs()).await;

    use overdrive_control_plane::error::{ControlPlaneError, DataplaneBootError};
    let err = result.expect_err("expected boot refusal when probe fault is injected");
    let display = format!("{err}");

    let is_probe = matches!(
        &err,
        ControlPlaneError::DataplaneBoot(DataplaneBootError::Probe {
            source: overdrive_core::traits::dataplane::DataplaneError::LoadFailed(_),
        }),
    );
    assert!(
        is_probe,
        "expected ControlPlaneError::DataplaneBoot(Probe {{ LoadFailed(_) }}); got: {err:?} \
         (display: {display})",
    );
    assert!(
        display.contains("probe: round-trip mismatch") || display.contains("probe: BACKEND_MAP"),
        "expected Display to contain 'probe: round-trip mismatch' OR 'probe: BACKEND_MAP'; \
         got: {display}",
    );

    // Structured-event assertion: at least one `health.startup.refused`
    // event names the probe reason.
    let events = collector.snapshot();
    let has_refused = events.iter().any(|e| {
        e.name == "health.startup.refused"
            && e.fields.get("reason").is_some_and(|r| r == "dataplane.probe")
    });
    assert!(
        has_refused,
        "expected at least one health.startup.refused event with reason=dataplane.probe; \
         observed events: {:?}",
        events.iter().map(|e| (e.name.clone(), e.fields.clone())).collect::<Vec<_>>(),
    );

    // The SERVICE_MAP pin must be gone because `EbpfDataplane::Drop`
    // fired when the boot path bailed via `?` on the probe error.
    assert!(
        !pin_path.exists(),
        "expected SERVICE_MAP pin at {} to be removed by EbpfDataplane::Drop after probe failure",
        pin_path.display(),
    );
}

/// S-BDB-15 — production boot succeeds when the Earned-Trust probe
/// round-trips a BACKEND_MAP sentinel. Boot reaches the HTTPS
/// listener bind (a successful `run_server` return after the probe
/// is the observable proof — the listener is already bound by the
/// time `ServerHandle` is handed back, per `server_lifecycle.rs`).
///
/// The probe's `BACKEND_MAP::get(u32::MAX, 0) == None` postcondition
/// is pinned by the `EbpfDataplane::probe` unit-test surface in
/// `crates/overdrive-dataplane/src/lib.rs`. At this Tier 3 scope, the
/// observable signal is "boot succeeds end-to-end with the live probe
/// path engaged" — anything else duplicates the unit assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn boot_succeeds_when_earned_trust_probe_round_trips_backend_map() {
    require_cap_net_admin();

    let fx = BootFixture::setup_veth_pair(Some("10.244.15.1/24")).expect("veth setup");
    let tmp = TempDir::new().expect("tmp");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = server_config_with(&fx, &data_dir, &cfg_dir);

    // No probe-fault injected → production probe path runs end-to-end.
    let handle: ServerHandle = overdrive_control_plane::run_server(config, test_cgroup_fs())
        .await
        .expect("run_server with probe success");

    handle.shutdown(std::time::Duration::from_secs(2)).await;
}
