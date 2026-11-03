//! Quint conformance driver QC-2 — `gateway_application` (public-ingress-gateway,
//! ADR-0222: drain and supersession).
//!
//! Replays `specs/quint/public-ingress-gateway/gateway_application.qnt`,
//! conformance main `app_conf` (the configuration of `gateway_application`
//! with `REC = true`, recording `conf_step`; D-UP-1), against the production
//! Application owner. The spec is referenced in place.
//!
//! The driver holds the production composition (`Composition` of
//! `held_probe_doubles`): `UnboundGatewayBuilder::new(..).await` ->
//! `into_builder()` -> `GatewayBuilder::start_with(deps, admission)` over a real
//! redb Route store, `SimClock` and `SimObservationStore` (core-trait Sims),
//! and the driver-local doubles of the four gateway-crate driven ports
//! (DG-7: the `overdrive-sim` gateway adapters implement the other copy of the
//! crate-local traits, so this module imports no gateway-trait Sim). The
//! injected `Arc<GatewayAdmission>` (DG-10), `GatewayRouteCommands` and `GatewayStatusReads` are fields of
//! the driver. The runtime is a paused single-thread tokio runtime; the owner
//! tasks run only while the driver yields (`block_on`).
//!
//! Action -> production call (feature-delta, "Conformance action maps for the
//! three older mains", `app_conf`):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | compose a fresh world (the previous one is shut down) on its own `127.<octet>.x.y:443` |
//! | `route_declare` | `GatewayRouteCommands::routes()` -> `PublicRouteSetHandle::apply` of a Route of a new generation; the owner has not yet run to idle |
//! | `route_withdraw` | `PublicRouteSetHandle::withdraw`, then yield until the owner is idle; its postcondition: nothing staged, nothing Current |
//! | `stage` | owner-autonomous, no driving-port call: yield until the owner has staged the live Route generation and its probe is held; assert that through `application_status().staged` and learn the minted identity of the held probe |
//! | `probe_result(tok, ok)` | release the entry held at `GatewayClientMtls::authenticate` for candidate `tok` by its minted identity (`ok`: usable `Ok(GatewayUpstream)`; otherwise `Err(Handshake)`), then yield until idle; releasing a discarded candidate's entry is a no-op |
//! | `retire(gid)` | owner-autonomous, no driving-port call: yield until `application_status().draining` no longer lists `gid` |
//! | `open_connection` | `GatewayAdmission::acquire_connection(now)` on the injected admission; the lease is kept |
//! | `close_connection(gid)` | drop the kept `GatewayConnectionLease` retaining generation `gid`; the owner has not yet run |
//!
//! The model has no stutter step in `app_conf` (R-7: `idle` is guarded
//! `not(REC)`), so no handler is a no-op.
//!
//! Projection (`State::from_driver`), per the pinned table:
//! `current_generation` / `draining_generations` <-
//! `GatewayStatusReads::application_status()` (`current`, `draining`). The model
//! variables `staged`, `inflight`, `leases`, `next_gid`, `route_on`, `route_gen`
//! and `conf_step` are model-only and are not projected (`route_on` and
//! `route_gen` are driver input state: the driver applies and withdraws the
//! Route itself). Reading the
//! status runs no owner step: a Sim observation read completes without
//! yielding.
//!
//! The one piece of driver bookkeeping is a naming translation, not a copy of
//! the protocol: the model numbers its candidates `1, 2, ...` in the order
//! `stage` stages them, and the owner names a generation by a 64-hex
//! `GatewayApplicationGenerationId`. At each `stage` the driver records the id
//! the owner reports as `staged` under the model's number (and the identity
//! of the probe held for it), and `from_driver` and the `close_connection` /
//! `retire` handlers translate through that record. Every compared value
//! (which generation is Current, which are Draining) is the owner's.
//!
//! Step decoding (D-UP-1): `Config::nondet =
//! &["app_conf::gateway_application::conf_step"]` (the qualified ITF name);
//! the state projection matches `app_conf::gateway_application::<var>`.
//!
//! Every named run that encodes a critical scenario is pinned with
//! `#[quint_test(max_samples = 1)]`; `#[quint_run]` replays randomized traces
//! (seed printed on failure). The tests are ignored until the Application
//! owner lands. The composition binds the real TCP/443 on its own loopback
//! address (R-6), so the lane runs as root (`cargo xtask metal run --`).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use anyhow::{anyhow, bail};
use itf::Value;
use overdrive_core::id::SpiffeId;
use overdrive_core::public_ingress::{GatewayApplicationGenerationId, Route, RouteId};
use overdrive_core::wall_clock::UnixInstant;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;

use super::held_probe_doubles::{
    Composition, ProbeOutcome, ROUTE_ID, next_ordinal, route, settle, wait_until,
};
use super::{GatewayAdmission, GatewayConnectionLease, GatewayRouteCommands, GatewayStatusReads};

/// The conformance main this driver replays.
const MAIN: &str = "app_conf";
/// The module that declares the compared variables.
const MODULE: &str = "gateway_application";
/// Second octets of this driver's bind addresses, one per test (nextest runs
/// each test in its own process; table in `held_probe_doubles`).
const REPLACE_VALID_OCTET: u8 = 63;
const ROUTE_WITHDRAW_DRAIN_OCTET: u8 = 65;
const LATE_PROBE_OCTET: u8 = 66;
const RANDOMIZED_TRACES_OCTET: u8 = 67;
/// One ordinal per composition of the process, so no two share an address.
static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

/// The model-number <-> owner-generation translation recorded at each `stage`.
#[derive(Default)]
struct GenerationNames {
    generation_of: BTreeMap<i64, GatewayApplicationGenerationId>,
    probe_of: BTreeMap<i64, SpiffeId>,
}

impl GenerationNames {
    fn model_of(&self, generation: &GatewayApplicationGenerationId) -> Result<i64> {
        self.generation_of
            .iter()
            .find_map(|(model, owner)| (owner == generation).then_some(*model))
            .ok_or_else(|| anyhow!("the owner reports generation {generation} no `stage` named"))
    }

    fn generation(&self, model: i64) -> Result<GatewayApplicationGenerationId> {
        self.generation_of
            .get(&model)
            .copied()
            .ok_or_else(|| anyhow!("model candidate {model} was never staged"))
    }

    fn probe(&self, model: i64) -> Result<SpiffeId> {
        self.probe_of
            .get(&model)
            .cloned()
            .ok_or_else(|| anyhow!("model candidate {model} has no held probe on record"))
    }
}

struct AppConfDriver {
    /// Second octet of every bind address this driver instance uses.
    octet: u8,
    /// The production owner's Route command surface of the current world.
    routes: GatewayRouteCommands,
    /// The production owner's redacted status reads of the current world.
    status: GatewayStatusReads,
    /// The admission injected into the owner and the listener (DG-10).
    admission: Arc<GatewayAdmission>,
    /// Admitted connections kept by `open_connection` until `close_connection`.
    leases: Vec<GatewayConnectionLease>,
    /// The composition: handle, held-probe registry, doubles, clock. Absent
    /// only while a world is being replaced.
    world: Option<Composition>,
    ids: GenerationNames,
    /// Stages performed so far; the model's `next_gid - 1`.
    stages: i64,
    /// Routes declared so far: makes each declared Route's content distinct
    /// (a distinct Route generation), nothing more.
    declared: u32,
    /// The Route most recently declared and not withdrawn.
    live_route: Option<Route>,
    /// Probes already attributed to a model candidate.
    claimed: BTreeSet<SpiffeId>,
    runtime: Runtime,
}

impl AppConfDriver {
    fn new(octet: u8) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("paused current-thread runtime");
        let world = runtime.block_on(Composition::start(octet, next_ordinal(&NEXT_ORDINAL)));
        Self {
            octet,
            routes: world.routes.clone(),
            status: world.status.clone(),
            admission: Arc::clone(&world.admission),
            leases: Vec::new(),
            world: Some(world),
            ids: GenerationNames::default(),
            stages: 0,
            declared: 0,
            live_route: None,
            claimed: BTreeSet::new(),
            runtime,
        }
    }

    fn world(&self) -> Result<&Composition> {
        self.world.as_ref().ok_or_else(|| anyhow!("the driver has no composed world"))
    }

    /// A fresh composition per trace; the previous world is shut down first.
    fn init(&mut self) {
        self.leases.clear();
        if let Some(previous) = self.world.take() {
            let report = self.runtime.block_on(previous.stop());
            drop(report);
        }
        let world =
            self.runtime.block_on(Composition::start(self.octet, next_ordinal(&NEXT_ORDINAL)));
        self.routes = world.routes.clone();
        self.status = world.status.clone();
        self.admission = Arc::clone(&world.admission);
        self.world = Some(world);
        self.ids = GenerationNames::default();
        self.stages = 0;
        self.declared = 0;
        self.live_route = None;
        self.claimed.clear();
    }

    fn route_declare(&mut self) -> Result {
        self.declared += 1;
        let route = route(&format!("/v{}", self.declared));
        self.runtime
            .block_on(self.routes.routes().apply(route.clone()))
            .map_err(|error| anyhow!("Route apply refused: {error}"))?;
        self.live_route = Some(route);
        Ok(())
    }

    fn route_withdraw(&mut self) -> Result {
        let route_id = RouteId::new(ROUTE_ID).map_err(|error| anyhow!("Route ID: {error}"))?;
        self.runtime
            .block_on(self.routes.routes().withdraw(route_id))
            .map_err(|error| anyhow!("Route withdraw refused: {error}"))?;
        self.live_route = None;
        let status = self.runtime.block_on(settle(&self.status)).map_err(|error| anyhow!(error))?;
        if let Some(row) = status.filter(|row| row.staged.is_some() || row.current.is_some()) {
            bail!("withdrawal left staged or Current in place: {row:?}");
        }
        Ok(())
    }

    /// Owner-autonomous: no driving-port call. Yield until the owner has
    /// staged the live Route generation and its probe is held; assert that
    /// through `application_status().staged`.
    fn stage(&mut self) -> Result {
        let live = self
            .live_route
            .as_ref()
            .ok_or_else(|| anyhow!("`stage` with no declared Route"))?
            .route_generation();
        let claimed = &self.claimed;
        let probes = &self.world()?.probes;
        let status = self
            .runtime
            .block_on(wait_until(
                &self.status,
                probes,
                "the live Route generation staged with its probe held",
                |status, held| {
                    status.is_some_and(|row| {
                        row.staged.as_ref().is_some_and(|staged| staged.route_generation == live)
                    }) && held.held_peers().iter().any(|peer| !claimed.contains(peer))
                },
            ))
            .map_err(|error| anyhow!(error))?;
        let staged = status
            .and_then(|row| row.staged)
            .ok_or_else(|| anyhow!("the status row lost its staged candidate"))?;
        let peer = probes
            .held_peers()
            .into_iter()
            .rev()
            .find(|peer| !claimed.contains(peer))
            .ok_or_else(|| anyhow!("no unclaimed held probe"))?;
        self.stages += 1;
        self.ids.generation_of.insert(self.stages, staged.application_generation);
        self.ids.probe_of.insert(self.stages, peer.clone());
        self.claimed.insert(peer);
        Ok(())
    }

    fn probe_result(&mut self, tok: i64, ok: bool) -> Result {
        let peer = self.ids.probe(tok)?;
        let outcome = if ok { ProbeOutcome::Usable } else { ProbeOutcome::Unusable };
        // A discarded candidate's entry may be gone: releasing it is a no-op.
        self.world()?.probes.release(&peer, outcome);
        self.runtime.block_on(settle(&self.status)).map_err(|error| anyhow!(error))?;
        Ok(())
    }

    /// Owner-autonomous: no driving-port call. Yield until the owner has
    /// retired `gid`.
    fn retire(&mut self, gid: i64) -> Result {
        let retiring = self.ids.generation(gid)?;
        let probes = &self.world()?.probes;
        self.runtime
            .block_on(wait_until(
                &self.status,
                probes,
                "the Draining generation retired",
                |status, _| {
                    status.is_none_or(|row| {
                        row.draining
                            .iter()
                            .all(|draining| draining.application_generation != retiring)
                    })
                },
            ))
            .map_err(|error| anyhow!(error))?;
        Ok(())
    }

    fn open_connection(&mut self) -> Result {
        let now = UnixInstant::from_clock(self.world()?.clock.as_ref());
        let lease = self
            .admission
            .acquire_connection(now)
            .map_err(|error| anyhow!("acquire_connection refused: {error:?}"))?;
        self.leases.push(lease);
        Ok(())
    }

    fn close_connection(&mut self, gid: i64) -> Result {
        let retained = self.ids.generation(gid)?;
        let position = self
            .leases
            .iter()
            .position(|lease| lease.snapshot().applied().application_generation() == retained)
            .ok_or_else(|| anyhow!("no kept lease retains model generation {gid}"))?;
        drop(self.leases.remove(position));
        Ok(())
    }
}

impl Drop for AppConfDriver {
    fn drop(&mut self) {
        self.leases.clear();
        if let Some(world) = self.world.take() {
            let report = self.runtime.block_on(world.stop());
            drop(report);
        }
    }
}

impl Driver for AppConfDriver {
    type State = ApplicationState;

    fn config() -> Config {
        Config { state: &[], nondet: &["app_conf::gateway_application::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            route_declare => self.route_declare()?,
            route_withdraw => self.route_withdraw()?,
            stage => self.stage()?,
            probe_result(tok: i64, ok: bool) => self.probe_result(tok, ok)?,
            retire(gid: i64) => self.retire(gid)?,
            open_connection => self.open_connection()?,
            close_connection(gid: i64) => self.close_connection(gid)?,
        })
    }
}

/// The compared `gateway_application` variables: the model's `st.current`
/// and `st.draining`, as generation numbers.
#[derive(Debug, PartialEq, Eq)]
struct ApplicationState {
    current_generation: Option<i64>,
    draining_generations: BTreeSet<i64>,
}

impl State<AppConfDriver> for ApplicationState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &AppConfDriver) -> Result<Self> {
        let status = driver
            .runtime
            .block_on(driver.status.application_status())
            .map_err(|error| anyhow!("application_status failed: {error}"))?;
        let Some(row) = status else {
            return Ok(Self { current_generation: None, draining_generations: BTreeSet::new() });
        };
        let current_generation = row
            .current
            .as_ref()
            .map(|current| driver.ids.model_of(&current.application_generation))
            .transpose()?;
        let draining_generations = row
            .draining
            .iter()
            .map(|draining| driver.ids.model_of(&draining.application_generation))
            .collect::<Result<BTreeSet<i64>>>()?;
        Ok(Self { current_generation, draining_generations })
    }
}

/// The spec side: `app_conf::gateway_application::st`, whose `current` and
/// `draining` are sets of `{ gid, rg }` objects.
impl ApplicationState {
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let Value::Record(record) = state else {
            return Err(ItfError::Shape {
                expected: "a record of variables",
                found: state.clone(),
            });
        };
        let qualified = format!("{MAIN}::{MODULE}::st");
        let st = record.get(&qualified).ok_or(ItfError::MissingVar(qualified))?;
        let Value::Record(st) = st else {
            return Err(ItfError::Shape { expected: "the state record `st`", found: st.clone() });
        };
        let gids = |field: &str| -> std::result::Result<BTreeSet<i64>, ItfError> {
            let set = st.get(field).ok_or_else(|| ItfError::MissingField(field.to_owned()))?;
            let Value::Set(objects) = set else {
                return Err(ItfError::Shape { expected: "a set of objects", found: set.clone() });
            };
            objects
                .iter()
                .map(|object| {
                    let Value::Record(object) = object else {
                        return Err(ItfError::Shape {
                            expected: "an object record",
                            found: object.clone(),
                        });
                    };
                    let gid = object
                        .get("gid")
                        .ok_or_else(|| ItfError::MissingField("gid".to_owned()))?;
                    match gid {
                        Value::Number(n) => Ok(*n),
                        Value::BigInt(big) => i64::try_from(big.get().clone()).map_err(|_| {
                            ItfError::Shape { expected: "a 64-bit int", found: gid.clone() }
                        }),
                        other => Err(ItfError::Shape { expected: "an int", found: other.clone() }),
                    }
                })
                .collect()
        };
        let current = gids("current")?;
        if current.len() > 1 {
            return Err(ItfError::Shape {
                expected: "at most one Current object",
                found: Value::Set(current.iter().map(|gid| Value::Number(*gid)).collect()),
            });
        }
        Ok(Self {
            current_generation: current.into_iter().next(),
            draining_generations: gids("draining")?,
        })
    }
}

impl<'de> Deserialize<'de> for ApplicationState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// Why a trace value does not have the shape the driver compares.
#[derive(Debug, thiserror::Error)]
enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("the trace's record has no `{0}` field")]
    MissingField(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

// ---------------------------------------------------------------------
// Named runs — one per critical scenario of the `Formal protocol model`
// section, then randomized traces.
// ---------------------------------------------------------------------

/// Supersede, drain, retire: the displaced generation serves its connection
/// until it closes, then retires.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_application.qnt",
    main = "app_conf",
    test = "replace_valid",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-gateway_application"]
fn gateway_application_replace_valid() -> impl Driver {
    AppConfDriver::new(REPLACE_VALID_OCTET)
}

/// Withdraw: Current drains and retires only after its connection closes; a
/// late probe result for the discarded staged candidate restores nothing.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_application.qnt",
    main = "app_conf",
    test = "route_withdraw_drain",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-gateway_application"]
fn gateway_application_route_withdraw_drain() -> impl Driver {
    AppConfDriver::new(ROUTE_WITHDRAW_DRAIN_OCTET)
}

/// A probe result for a superseded staged candidate is discarded: the newer
/// candidate stays staged and nothing is promoted until its own result.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_application.qnt",
    main = "app_conf",
    test = "late_probe_after_supersession",
    max_samples = 1
)]
#[ignore = "pending DELIVER quint-conformance-gateway_application"]
fn gateway_application_late_probe_after_supersession() -> impl Driver {
    AppConfDriver::new(LATE_PROBE_OCTET)
}

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(
    spec = "../../specs/quint/public-ingress-gateway/gateway_application.qnt",
    main = "app_conf"
)]
#[ignore = "pending DELIVER quint-conformance-gateway_application"]
fn gateway_application_app_conf_randomized_traces() -> impl Driver {
    AppConfDriver::new(RANDOMIZED_TRACES_OCTET)
}
