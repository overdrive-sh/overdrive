//! Quint conformance driver QC-3 — `selection_receipt` (public-ingress-gateway,
//! ADR-0232 / ADR-0233 / ADR-0241).
//!
//! Replays `specs/quint/public-ingress-gateway/selection_receipt.qnt`,
//! conformance main `receipt_conf` (the configuration of `selection_receipt`
//! with `REC = true`, recording `conf_step`; D-PASS7-2), against the real
//! [`EbpfDataplane`] on native metal with the real gateway selection
//! `cgroup/connect4` arm loaded. The spec is referenced in place. No `Sim*`
//! adapter is the owner or a projection source.
//!
//! Action -> owner driving port (feature-delta, "Conformance action maps for the
//! three older mains", `receipt_conf`; the model has no `idle` step, R-7):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | rebuild the owner on a fresh instance (nothing survives) |
//! | `update_apply(key, sidx)` | `Dataplane::update_service(frontend(key), backends(sidx))`, which must be accepted |
//! | `update_rejected(key, sidx)` | `Dataplane::update_service` of a backend set that re-leases a live `BackendId`'s address to another allocation; it must return `DataplaneError::BackendIdentityConflict` |
//! | `open(key)` | `GatewayConnectDataplane::open_upstream(ServiceKey)` |
//! | `kernel_connect_det(c)` | `connect(2)` on the socket from `PreparedUpstream::into_parts` toward the Service frontend, with a bounded wait (the connect result is not asserted: the hook has already written the receipt) |
//! | `take(c, c)` | `GatewayConnectDataplane::take_receipt(&intent)` |
//! | `read_identity(c)` | `GatewayConnectDataplane::selected_backend_identity(BackendId)` |
//! | `cleanup(c)` | `GatewayConnectDataplane::cleanup(&intent)` |
//! | `restart` | drop and re-create the `EbpfDataplane` on the same pin directory (SR-2) |
//!
//! Projection (`State::from_driver`), per the pinned table:
//! `ident[id]` <- `selected_backend_identity(BackendId)` for each `BackendId`
//! the allocator can have minted (`Ok` -> bound, `BackendIdentityMissing` ->
//! absent); `intents` <- `live_intent_count()`; `receipts` <- the length of
//! `GatewayConnectCleanup::live_receipts()`. The model variables `outer`, `resv`, `infl`,
//! `guard`, `debt`, `g`, `ckey`, `upd`, `next_cookie`, `conn`, `memo`,
//! `next_bid` are model-only and are not projected (no owner accessor is
//! pinned for them).
//!
//! The spec's `taken` (the receipt a connector consumed) is a port-return
//! check, not owner state, and is NOT part of the compared State (a
//! driver-maintained record of it would compare the driver with the model). It
//! is asserted at the `take` handler, as a direct assertion that fails the
//! step: the consumed receipt must be bound to the registered intent's
//! `ServiceKey` and cookie (`ReceiptMatchesIntent`). The spec has no `idres`
//! variable (R-33): `read_identity` is a step whose handler only calls
//! `selected_backend_identity` for the cookie's Selected backend and asserts
//! that the first read is present and that a second read agrees. The
//! identities are compared through the projected `ident` map (every probed
//! `BackendId`).
//!
//! Step decoding (D-UP-1): `Config::nondet =
//! &["receipt_conf::selection_receipt::conf_step"]`; the state projection
//! matches `receipt_conf::selection_receipt::<var>`.
//!
//! Every named run bound to this main is pinned with
//! `#[quint_test(max_samples = 1)]`; `#[quint_run]` replays randomized traces
//! (seed printed on failure). The tests are ignored until the gateway selection
//! program and the owner land. The lane runs as root through
//! `cargo xtask metal run --` (bpffs, cgroup v2, a veth pair for the loader's
//! XDP attach).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail};
use itf::Value;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::ServiceKey;
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayConnectIntent, GatewaySelectionReceipt,
};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

use crate::EbpfDataplane;

/// The conformance main this driver replays.
const MAIN: &str = "receipt_conf";
/// The module that declares the compared variables.
const MODULE: &str = "selection_receipt";
/// Highest `BackendId` the projection probes. The conformance catalogue has two
/// backend addresses and at most three accepted `update_service` calls
/// (`MAXUPD = 3`), so no more than this many ids are ever minted.
const PROBED_BACKEND_IDS: u32 = 8;
/// Bound on the driver's `connect(2)`: an absent listener must not stall it.
const CONNECT_BOUND: Duration = Duration::from_secs(2);
/// Port of every model Service frontend.
const FRONTEND_PORT: u16 = 8080;

/// One ordinal per owner built in the process, so veth names never collide.
static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

/// An RAII veth pair the loader attaches its XDP programs to.
struct Veth {
    host: String,
    peer: String,
}

impl Veth {
    fn create() -> Self {
        let ordinal = NEXT_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let host = format!("qh{}x{ordinal}", std::process::id() % 100_000);
        let peer = format!("qp{}x{ordinal}", std::process::id() % 100_000);
        // A stale pair of the same name is removed first; "no such device" is the
        // expected answer, so only a failure to run `ip(8)` at all is a precondition failure.
        delete_link(&host).expect("spawn ip(8) to remove a stale veth pair");
        for args in [
            vec!["link", "add", host.as_str(), "type", "veth", "peer", "name", peer.as_str()],
            vec!["link", "set", host.as_str(), "up"],
            vec!["link", "set", peer.as_str(), "up"],
        ] {
            let status = Command::new("ip").args(&args).status().expect("spawn ip(8)"); // subprocess-ok: test-owned veth fixture, Tier-3 only
            assert!(status.success(), "ip {args:?} failed: needs CAP_NET_ADMIN (run as root)");
        }
        Self { host, peer }
    }
}

/// Delete a link by name; "no such device" is a successful output, only a
/// failure to run `ip(8)` is an error.
fn delete_link(name: &str) -> std::io::Result<std::process::Output> {
    Command::new("ip").args(["link", "del", name]).output() // subprocess-ok: test-owned veth fixture, Tier-3 only
}

impl Drop for Veth {
    fn drop(&mut self) {
        if let Err(error) = delete_link(&self.host) {
            let _reported = writeln!(
                std::io::stderr(),
                "could not run ip(8) to remove veth {}: {error}",
                self.host
            );
        }
    }
}

/// The test-owned bpffs pin directory; removed on drop.
struct PinDir(PathBuf);

impl PinDir {
    fn create() -> Self {
        let path = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-qc3-{}", std::process::id()));
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                panic!("remove the stale bpffs pin directory {}: {error}", path.display())
            }
        }
        std::fs::create_dir_all(&path).expect("create the bpffs pin directory");
        Self(path)
    }
}

impl Drop for PinDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            let _reported = writeln!(
                std::io::stderr(),
                "could not remove the bpffs pin directory {}: {error}",
                self.0.display()
            );
        }
    }
}

/// The model Service key `key` (1 or 2) as a real frontend.
fn frontend(key: i64) -> ServiceFrontend {
    let octet = u8::try_from(key).expect("model Service keys are 1 and 2");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(198, 18, 250, octet))).expect("IPv4 VIP");
    ServiceFrontend::new(vip, NonZeroU16::new(FRONTEND_PORT).expect("nonzero port"), Proto::Tcp)
        .expect("IPv4 frontend")
}

/// The model allocation `alloc` (1 or 2) as a real workload identity.
fn alloc_identity(alloc: i64) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/conf/alloc/a{alloc}"))
        .expect("allocation SpiffeId")
}

/// The model catalogue of `update_service` backend sets (`CATALOG`): index 0 is
/// empty; 1 is address 1 / alloc 1; 4 re-leases address 1 to alloc 2; 5 is
/// address 2 / alloc 2. All are healthy (the arm ignores `healthy`, R-2).
fn backends(sidx: i64) -> Result<Vec<Backend>> {
    let backend = |addr: u8, alloc: i64| Backend {
        alloc: alloc_identity(alloc),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, addr)), 9000 + u16::from(addr)),
        weight: 1,
        healthy: true,
    };
    Ok(match sidx {
        0 => Vec::new(),
        1 => vec![backend(1, 1)],
        4 => vec![backend(1, 2)],
        5 => vec![backend(2, 2)],
        other => bail!("catalogue index {other} is not in the conformance catalogue"),
    })
}

/// A registered upstream socket the driver holds for one model cookie.
struct HeldSocket {
    key: i64,
    intent: GatewayConnectIntent,
    /// Present until `kernel_connect_det` consumes it.
    socket: Option<tokio::net::TcpSocket>,
    /// The connected stream, kept alive until `cleanup` or `restart`.
    stream: Option<TcpStream>,
    /// The `BackendId` of the receipt `take` consumed.
    selected: Option<BackendId>,
}

struct ReceiptConfDriver {
    /// The production owner: the real dataplane with the gateway selection arm.
    dataplane: EbpfDataplane,
    /// The veth pair the owner's loader attached to.
    veth: Veth,
    /// The pin directory shared by every instance of this driver (SR-2).
    pins: PinDir,
    /// Model cookies are minted in `open` order: 1, 2, ...
    held: BTreeMap<i64, HeldSocket>,
    opened: i64,
    runtime: Runtime,
}

impl ReceiptConfDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let pins = PinDir::create();
        let veth = Veth::create();
        let dataplane = Self::build_owner(&veth, &pins);
        Self { dataplane, veth, pins, held: BTreeMap::new(), opened: 0, runtime }
    }

    fn build_owner(veth: &Veth, pins: &PinDir) -> EbpfDataplane {
        EbpfDataplane::new_with_pin_dir(
            &veth.host,
            &veth.peer,
            &pins.0,
            std::path::Path::new("/sys/fs/cgroup"),
        )
        .expect("EbpfDataplane::new_with_pin_dir with the cgroup attach")
    }

    /// Drop the owner and build a new instance on the same pin directory.
    /// The new instance is built beside the old one on a fresh veth pair (the
    /// loader owns the XDP slot of an interface), then the old one is dropped.
    fn recreate_owner(&mut self) {
        self.held.clear();
        let veth = Veth::create();
        let owner = Self::build_owner(&veth, &self.pins);
        self.dataplane = owner;
        self.veth = veth;
    }

    fn init(&mut self) {
        self.recreate_owner();
        self.opened = 0;
    }

    fn update_apply(&self, key: i64, sidx: i64) -> Result {
        self.runtime
            .block_on(self.dataplane.update_service(frontend(key), backends(sidx)?))
            .map_err(|error| anyhow!("update_service must be accepted: {error}"))
    }

    fn update_rejected(&self, key: i64, sidx: i64) -> Result {
        match self.runtime.block_on(self.dataplane.update_service(frontend(key), backends(sidx)?)) {
            Ok(()) => bail!("update_service of a re-leased address must be rejected, it succeeded"),
            Err(DataplaneError::BackendIdentityConflict { .. }) => Ok(()),
            Err(other) => bail!(
                "update_service of a re-leased address must fail with BackendIdentityConflict, got {other}"
            ),
        }
    }

    fn open(&mut self, key: i64) -> Result {
        let _guard = self.runtime.enter();
        let prepared = self
            .dataplane
            .open_upstream(ServiceKey::from_frontend(frontend(key)))
            .map_err(|error| anyhow!("open_upstream failed: {error}"))?;
        let (socket, intent) = prepared.into_parts();
        self.opened += 1;
        self.held.insert(
            self.opened,
            HeldSocket { key, intent, socket: Some(socket), stream: None, selected: None },
        );
        Ok(())
    }

    fn kernel_connect_det(&mut self, cookie: i64) -> Result {
        let held = self.held.get_mut(&cookie).ok_or_else(|| anyhow!("cookie {cookie} not open"))?;
        let socket = held.socket.take().ok_or_else(|| anyhow!("cookie {cookie} already dialed"))?;
        let target = SocketAddr::V4(SocketAddrV4::new(frontend(held.key).vip_v4(), FRONTEND_PORT));
        let outcome = self
            .runtime
            .block_on(async { tokio::time::timeout(CONNECT_BOUND, socket.connect(target)).await });
        // The connect result is not asserted: the hook wrote the receipt before it
        // allowed or denied the connect; a refused or timed-out dial is the model's
        // failed dial.
        held.stream = outcome.ok().and_then(std::result::Result::ok);
        Ok(())
    }

    fn take(&mut self, cookie: i64) -> Result {
        let held = self.held.get_mut(&cookie).ok_or_else(|| anyhow!("cookie {cookie} not open"))?;
        let receipt = self
            .dataplane
            .take_receipt(&held.intent)
            .map_err(|error| anyhow!("take_receipt failed: {error}"))?;
        let expected_key = ServiceKey::from_frontend(frontend(held.key));
        match receipt {
            GatewaySelectionReceipt::Selected { backend_id, service_key, .. } => {
                if service_key != expected_key {
                    bail!(
                        "cookie {cookie}: the consumed receipt is bound to {service_key:?}, not the registered {expected_key:?}"
                    );
                }
                held.selected = Some(backend_id);
            }
            GatewaySelectionReceipt::NoBackend { service_key, .. } => {
                if service_key != expected_key {
                    bail!(
                        "cookie {cookie}: the consumed receipt is bound to {service_key:?}, not the registered {expected_key:?}"
                    );
                }
            }
        }
        Ok(())
    }

    fn read_identity(&self, cookie: i64) -> Result {
        let backend = self
            .held
            .get(&cookie)
            .and_then(|held| held.selected)
            .ok_or_else(|| anyhow!("cookie {cookie} has no Selected receipt to resolve"))?;
        let first = self.identity_of(backend)?;
        if first.is_none() {
            bail!(
                "selected_backend_identity({backend}) answered no identity for a Selected receipt"
            );
        }
        let second = self.identity_of(backend)?;
        if first != second {
            bail!("selected_backend_identity({backend}) answered {first:?} then {second:?}");
        }
        Ok(())
    }

    fn cleanup(&mut self, cookie: i64) -> Result {
        let held = self.held.remove(&cookie).ok_or_else(|| anyhow!("cookie {cookie} not open"))?;
        self.dataplane.cleanup(&held.intent).map_err(|error| anyhow!("cleanup failed: {error}"))?;
        drop(held);
        Ok(())
    }

    fn restart(&mut self) {
        self.recreate_owner();
    }

    /// The model allocation bound to `backend`, `None` when the owner answers
    /// `BackendIdentityMissing`.
    fn identity_of(&self, backend: BackendId) -> Result<Option<i64>> {
        match self.dataplane.selected_backend_identity(backend) {
            Ok(spiffe) => [1, 2]
                .into_iter()
                .find(|alloc| alloc_identity(*alloc) == spiffe)
                .map(Some)
                .ok_or_else(|| anyhow!("the owner bound {backend} to an unknown identity")),
            Err(GatewayConnectDataplaneError::BackendIdentityMissing { .. }) => Ok(None),
            Err(other) => bail!("selected_backend_identity({backend}) failed: {other}"),
        }
    }
}

impl Driver for ReceiptConfDriver {
    type State = ReceiptState;

    fn config() -> Config {
        Config { state: &[], nondet: &["receipt_conf::selection_receipt::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            update_apply(key: i64, sidx: i64) => self.update_apply(key, sidx)?,
            update_rejected(key: i64, sidx: i64) => self.update_rejected(key, sidx)?,
            open(key: i64) => self.open(key)?,
            kernel_connect_det(c: i64) => self.kernel_connect_det(c)?,
            take(c: i64, r: i64) => {
                if c != r {
                    bail!("a mismatched receipt cannot be requested through take_receipt(&intent)");
                }
                self.take(c)?;
            },
            read_identity(c: i64) => self.read_identity(c)?,
            cleanup(c: i64) => self.cleanup(c)?,
            restart => self.restart(),
        })
    }
}

/// The compared variables: the model's `ident`, `intents`, `receipts`.
#[derive(Debug, PartialEq, Eq)]
struct ReceiptState {
    ident: BTreeMap<i64, i64>,
    intents: usize,
    receipts: usize,
}

impl State<ReceiptConfDriver> for ReceiptState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &ReceiptConfDriver) -> Result<Self> {
        let mut ident = BTreeMap::new();
        for raw in 1..=PROBED_BACKEND_IDS {
            let backend = BackendId::new(raw).map_err(|error| anyhow!("BackendId: {error}"))?;
            if let Some(alloc) = driver.identity_of(backend)? {
                ident.insert(i64::from(raw), alloc);
            }
        }
        let intents = driver
            .dataplane
            .live_intent_count()
            .map_err(|error| anyhow!("live_intent_count failed: {error}"))?;
        let receipts = driver
            .dataplane
            .live_receipts()
            .map_err(|error| anyhow!("live_receipts failed: {error}"))?;
        Ok(Self { ident, intents: intents as usize, receipts: receipts.len() })
    }
}

/// Why a trace value does not have the shape the driver compares.
#[derive(Debug, thiserror::Error)]
enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

fn int(value: &Value) -> std::result::Result<i64, ItfError> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::BigInt(big) => i64::try_from(big.get().clone())
            .map_err(|_| ItfError::Shape { expected: "a 64-bit int", found: value.clone() }),
        other => Err(ItfError::Shape { expected: "an int", found: other.clone() }),
    }
}

fn entries(value: &Value) -> std::result::Result<Vec<(i64, &Value)>, ItfError> {
    let Value::Map(map) = value else {
        return Err(ItfError::Shape { expected: "a map", found: value.clone() });
    };
    map.iter().map(|(key, value)| Ok((int(key)?, value))).collect()
}

impl ReceiptState {
    /// The spec side: the `receipt_conf::selection_receipt::*` variables.
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let Value::Record(record) = state else {
            return Err(ItfError::Shape {
                expected: "a record of variables",
                found: state.clone(),
            });
        };
        let var = |name: &str| -> std::result::Result<&Value, ItfError> {
            let qualified = format!("{MAIN}::{MODULE}::{name}");
            record.get(&qualified).ok_or(ItfError::MissingVar(qualified))
        };
        let ident = entries(var("ident")?)?
            .into_iter()
            .map(|(id, alloc)| Ok((id, int(alloc)?)))
            .collect::<std::result::Result<BTreeMap<_, _>, ItfError>>()?;
        Ok(Self {
            ident,
            intents: entries(var("intents")?)?.len(),
            receipts: entries(var("receipts")?)?.len(),
        })
    }
}

impl<'de> Deserialize<'de> for ReceiptState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------
// Named runs bound to `receipt_conf`, then randomized traces.
// ---------------------------------------------------------------------

/// Commit a generation, select through a registered socket, read the
/// committed identity, clean up.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/selection_receipt.qnt",
    main = "receipt_conf",
    test = "commit_then_select",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-selection_receipt"]
fn selection_receipt_commit_then_select() -> impl Driver {
    ReceiptConfDriver::new()
}

/// An address re-leased to another allocation is rejected before the swap; the
/// applied generation and its identity are unchanged.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/selection_receipt.qnt",
    main = "receipt_conf",
    test = "conflicting_alloc_rejected",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-selection_receipt"]
fn selection_receipt_conflicting_alloc_rejected() -> impl Driver {
    ReceiptConfDriver::new()
}

/// A restart destroys intents, receipts, associations and the allocator memo;
/// the new instance mints identities again from the first id.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/selection_receipt.qnt",
    main = "receipt_conf",
    test = "restart_destroys_receipts",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-selection_receipt"]
fn selection_receipt_restart_destroys_receipts() -> impl Driver {
    ReceiptConfDriver::new()
}

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(
    spec = "../../specs/quint/public-ingress-gateway/selection_receipt.qnt",
    main = "receipt_conf"
)]
#[ignore = "pending DELIVER quint-conformance-selection_receipt"]
fn selection_receipt_receipt_conf_randomized_traces() -> impl Driver {
    ReceiptConfDriver::new()
}
