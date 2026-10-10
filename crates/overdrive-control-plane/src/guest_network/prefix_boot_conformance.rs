//! Quint conformance driver — `prefix_boot` (P4-S17; QPROJ-1, QPROJ-7,
//! DG-15).
//!
//! Replays `specs/quint/guest-flow-owner/prefix_boot.qnt` (instances
//! `boot_ok`, `boot_faulty`, `boot_live`; invariant 6, D8a-ROUTE,
//! D8a-LOAD-SWAP, U-4) against the production owner
//! `HostSharedGuestNetworkOwner::with_flow_io(ports, SimGuestFlowIo)`, whose
//! boot is `SharedGuestNetworkOwner::converge_shared`. The spec is referenced
//! in place; the driver re-implements no rule of the protocol.
//!
//! Action → driving port / seam stimulus (feature-delta § "Per-module
//! projection and action map", `prefix_boot`):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a node never booted (`SimGuestNode::new`), no `serve` |
//! | `Restart(b)` | `serve` starts binary `b`: a new seam over the persisted node with steering program `b`, and a new owner over it (`with_flow_io`) |
//! | `BootStep` | at `start`: call `converge_shared` (boot steps 1–8 are one step; the owner's forwarder load and listener binds are released) and run it until it parks on the held steering `Load`; otherwise release the held steering / route effect |
//! | `Fail` | fail the held effect: `Load` (verifier rejection), the swap (`Update` on an adopted link, `Attach` on a first attach), `Pin`, or the route write; `converge_shared` returns `Err` and `serve` exits (a refusal) |
//! | `Crash` | drop the owner and its pending `converge_shared`; the process's objects die with it (an unpinned link vanishes) |
//!
//! Projection: `link`, `prog`, `route` ← `SimGuestFlowIo::steering_observation()`
//! (QPROJ-7 sim arbiter, paired Tier-3 R5-24, R5-34, R5-25); `phase` ← the
//! caller's own observation of `converge_shared` (QPROJ-1 source (d): not
//! called / pending / returned `Ok`) combined with
//! `SimGuestFlowIo::steering_effects()` of this `serve`'s seam (the step after
//! the last `Ok` effect). `up`, `stop`, `bin` are the driver's stimulus;
//! `loaded`, `swapOk`, `adopted` model-internal; `estab`, `g` ghost;
//! `failLeft`, `crashLeft` stimulus — not compared.
//!
//! What the driver checks, and what it does not. `converge_shared`'s steering
//! step calls `GuestFlowIo::converge_steering` once; the sim executes that
//! call as its own `Load`, then `Attach` (an `Update` of an adopted link),
//! then — for a first attach only — `Pin` effects, and enforces the adapter
//! rules itself: an adopted link is never detached, and a pin is taken only
//! on a first attach. So this driver checks the OWNER's ordering (load and
//! swap before the route, the route never local without steering, a refusal
//! on any failed step, convergence after a crash at any step); the
//! Load/Attach/Pin sequencing inside one `converge_steering` and the
//! never-detach / pin-only-on-first-attach rules rest on the real adapter's
//! Tier-3 cells in `crates/overdrive-dataplane/tests/integration/vsock_guest_steering.rs`
//! (P4-S4, R5-24 / R5-34): `a_first_attach_pins_a_link_that_outlives_the_handle`
//! and `boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches`.
//!
//! The initial state. quint 0.32 `run --n-traces` labels the initial state of
//! every `boot_live` trace `step` instead of `init` (index 0 only, with the
//! `init` state's values; never mid-trace — checked over seeds 1, 2, 3, 11, 42
//! × 100 traces of each instance). The driver maps that `step` to `init`. It
//! cannot restrict the mapping to "before `init` ran": quint-connect replays
//! every trace through ONE driver, so the `step` opening trace 2+ arrives after
//! the previous trace's `init` and last action, in whatever state that trace
//! ended (`serve` up or down). A `step` label anywhere else resets the node and
//! diverges at the per-step state comparison unless the spec's state there is
//! the initial state itself.
//!
//! The spec defines no Quint `run` declarations, so there is no named run for
//! `#[quint_test]`; the three instances replay randomized traces through
//! `#[quint_run]` (the seed is printed on failure).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "a conformance driver asserts its own preconditions; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use quint_connect::{Driver, Result, State, Step, quint_run, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use super::flow_io::GuestFlowIo;
use super::flow_io::sim::{
    SimEffect, SimEffectId, SimEffectKind, SimEffectTarget, SimFailure, SimGuestFlowIo,
    SimGuestNode, SimPrefixRoute, SimProgramVersion, SimSteeringEffect, SimSteeringFailure,
    SimSteeringLink,
};
use super::prefix_boot_acceptance::{disjoint_vip, owner_ports};
use super::{HostSharedGuestNetworkOwner, SharedGuestNetworkOwner};

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: &'static str },
}

/// The caller's own observation of the one `converge_shared` call of the
/// current `serve` (QPROJ-1 source (d)).
enum BootCall {
    /// `serve` runs; `converge_shared` not called yet.
    NotCalled,
    /// Called; not returned.
    Pending(JoinHandle<super::Result<()>>),
    /// Returned `Ok` (boot opened).
    Opened,
}

/// The production owner of the running `serve`, with its boot call.
struct Serve {
    owner: Arc<HostSharedGuestNetworkOwner>,
    boot: BootCall,
}

struct PrefixBootDriver {
    runtime: Runtime,
    /// Node-global sim objects; they persist across `serve` processes.
    node: Arc<SimGuestNode>,
    /// The seam of the current (or, while down, the last) `serve` process.
    io: Arc<SimGuestFlowIo>,
    /// The production owner of the running `serve`; `None` while down.
    serve: Option<Serve>,
}

impl PrefixBootDriver {
    fn new() -> Self {
        let node = SimGuestNode::new();
        let io = SimGuestFlowIo::new(&node, SimProgramVersion::new("none"));
        io.exit_process();
        Self {
            // Paused time: `SimGuestFlowIo::settled` detects idleness through
            // the paused runtime's auto-advance (the owner reads time only
            // through its injected `SimClock`).
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .start_paused(true)
                .build()
                .expect("current-thread runtime"),
            node,
            io,
            serve: None,
        }
    }

    /// `init`: a node never booted and no `serve`.
    fn init(&mut self) {
        self.crash_serve();
        self.node = SimGuestNode::new();
        self.io = SimGuestFlowIo::new(&self.node, SimProgramVersion::new("none"));
        self.io.exit_process();
    }

    /// `Restart(b)`: `serve` starts binary `b` over the persisted node.
    fn restart(&mut self, binary: &str) -> std::result::Result<(), DriverError> {
        if self.serve.is_some() {
            return Err(DriverError::NotEnabled { action: "Restart", why: "serve is up" });
        }
        let io = SimGuestFlowIo::new(&self.node, SimProgramVersion::new(binary));
        io.auto_release_all();
        for kind in [
            SimEffectKind::SteeringLoad,
            SimEffectKind::SteeringAttach,
            SimEffectKind::SteeringPin,
            SimEffectKind::PrefixRoute,
        ] {
            io.hold(kind);
        }
        let seam: Arc<dyn GuestFlowIo> = Arc::clone(&io) as Arc<dyn GuestFlowIo>;
        let owner = Arc::new(HostSharedGuestNetworkOwner::with_flow_io(
            owner_ports(&disjoint_vip(), &[]),
            seam,
        ));
        self.io = io;
        self.serve = Some(Serve { owner, boot: BootCall::NotCalled });
        Ok(())
    }

    /// `BootStep`: call `converge_shared`, or release its held effect.
    fn boot_step(&mut self) -> std::result::Result<(), DriverError> {
        let Some(serve) = self.serve.as_mut() else {
            return Err(DriverError::NotEnabled { action: "BootStep", why: "serve is down" });
        };
        match &serve.boot {
            BootCall::NotCalled => {
                let owner = Arc::clone(&serve.owner);
                serve.boot = BootCall::Pending(
                    self.runtime.spawn(async move { owner.converge_shared().await }),
                );
            }
            BootCall::Pending(_) => {
                let (id, _effect) = self.held_boot_effect("BootStep")?;
                self.io.release(id);
            }
            BootCall::Opened => {
                return Err(DriverError::NotEnabled { action: "BootStep", why: "boot opened" });
            }
        }
        self.settle();
        Ok(())
    }

    /// `Fail`: the held boot effect fails; `serve` exits on the refusal.
    fn fail(&mut self) -> std::result::Result<(), DriverError> {
        let (id, effect) = self.held_boot_effect("Fail")?;
        let failure = match (effect.kind, effect.target) {
            (SimEffectKind::SteeringLoad, _) => SimFailure::Steering(SimSteeringFailure::Load),
            (SimEffectKind::SteeringAttach, SimEffectTarget::Adopted(true)) => {
                SimFailure::Steering(SimSteeringFailure::Update)
            }
            (SimEffectKind::SteeringAttach, _) => SimFailure::Steering(SimSteeringFailure::Attach),
            (SimEffectKind::SteeringPin, _) => SimFailure::Steering(SimSteeringFailure::Pin),
            (_, _) => SimFailure::Netlink,
        };
        self.io.fail(id, failure);
        self.settle();
        Ok(())
    }

    /// `Crash`: `serve` dies with its pending boot.
    fn crash(&mut self) -> std::result::Result<(), DriverError> {
        if self.serve.is_none() {
            return Err(DriverError::NotEnabled { action: "Crash", why: "serve is down" });
        }
        self.crash_serve();
        Ok(())
    }

    fn crash_serve(&mut self) {
        if let Some(serve) = self.serve.take() {
            if let BootCall::Pending(task) = serve.boot {
                task.abort();
                let _cancelled = self.runtime.block_on(task);
            }
            drop(serve.owner);
            self.io.exit_process();
        }
    }

    /// The one held steering / route effect the pending boot is parked on.
    fn held_boot_effect(
        &self,
        action: &'static str,
    ) -> std::result::Result<(SimEffectId, SimEffect), DriverError> {
        let held = self.io.held();
        match held.as_slice() {
            [one] => Ok(*one),
            [] => Err(DriverError::NotEnabled { action, why: "the boot holds no effect" }),
            _ => Err(DriverError::NotEnabled { action, why: "the boot holds several effects" }),
        }
    }

    /// Runs the owner until it is parked on the seam, then records how its
    /// boot call stands; a refusal (`Err`) ends `serve`.
    fn settle(&mut self) {
        self.runtime.block_on(self.io.settled());
        let Some(serve) = self.serve.as_mut() else { return };
        let BootCall::Pending(task) = &serve.boot else { return };
        if !task.is_finished() {
            return;
        }
        let BootCall::Pending(task) = std::mem::replace(&mut serve.boot, BootCall::NotCalled)
        else {
            unreachable!("the boot call was pending just above");
        };
        match self.runtime.block_on(task) {
            Ok(Ok(())) => serve.boot = BootCall::Opened,
            Ok(Err(_refused)) => {
                self.serve = None;
                self.io.exit_process();
            }
            Err(join) => std::panic::resume_unwind(join.into_panic()),
        }
    }
}

impl Driver for PrefixBootDriver {
    type State = PrefixBootState;

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            // quint 0.32 labels a trace's initial state `step` (module doc,
            // "The initial state").
            step => self.init(),
            Restart(b: String) => self.restart(&b)?,
            BootStep => self.boot_step()?,
            Fail => self.fail()?,
            Crash => self.crash()?,
        })
    }
}

/// `prefix_boot`'s `phase` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Load,
    Swap,
    Pin,
    Route,
    Open,
}

impl Phase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Load => "load",
            Self::Swap => "swap",
            Self::Pin => "pin",
            Self::Route => "route",
            Self::Open => "open",
        }
    }
}

/// The compared `prefix_boot` variables (Quint strings; ITF decodes a
/// string-valued variable as a string, not as an enum).
#[derive(Debug, PartialEq, Eq)]
struct PrefixBootState {
    phase: String,
    link: String,
    prog: String,
    route: String,
}

/// The trace names each variable `<instance>::prefix_boot::<var>`; the
/// driver compares by the variable's own name, whichever instance ran.
impl<'de> Deserialize<'de> for PrefixBootState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let vars = deserializer.deserialize_any(TraceVarsVisitor)?;
        let var = |name: &str| -> std::result::Result<String, D::Error> {
            match vars.iter().find(|(key, _)| key.rsplit("::").next() == Some(name)) {
                Some((_, TraceVar::Str(value))) => Ok(value.clone()),
                Some((_, TraceVar::Other)) => {
                    Err(D::Error::custom(format!("the trace's `{name}` is not a string")))
                }
                None => Err(D::Error::custom(format!("the trace has no `{name}` variable"))),
            }
        };
        Ok(Self {
            phase: var("phase")?,
            link: var("link")?,
            prog: var("prog")?,
            route: var("route")?,
        })
    }
}

/// The trace state: a record of variables (ITF decodes a record only through
/// `deserialize_any` / `deserialize_struct`, never `deserialize_map`).
struct TraceVarsVisitor;

impl<'de> serde::de::Visitor<'de> for TraceVarsVisitor {
    type Value = BTreeMap<String, TraceVar>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a record of trace variables")
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut vars = BTreeMap::new();
        while let Some((name, value)) = map.next_entry::<String, TraceVar>()? {
            vars.insert(name, value);
        }
        Ok(vars)
    }
}

/// One trace variable: a string, or a value this driver does not compare.
enum TraceVar {
    Str(String),
    Other,
}

impl<'de> Deserialize<'de> for TraceVar {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(TraceVarVisitor)
    }
}

struct TraceVarVisitor;

impl<'de> serde::de::Visitor<'de> for TraceVarVisitor {
    type Value = TraceVar;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an ITF value")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Str(value.to_owned()))
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Str(value))
    }
    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Other)
    }
    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Other)
    }
    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Other)
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<TraceVar, E> {
        Ok(TraceVar::Other)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<TraceVar, A::Error> {
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok(TraceVar::Other)
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<TraceVar, A::Error> {
        while map.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?.is_some() {}
        Ok(TraceVar::Other)
    }
}

impl State<PrefixBootDriver> for PrefixBootState {
    fn from_driver(driver: &PrefixBootDriver) -> Result<Self> {
        // conformance-lint: sim-arbiter-ok node-global pinned link / program / tagged route arbiter, paired Tier-3 R5-24, R5-34, R5-25
        let steering = driver.io.steering_observation();
        // conformance-lint: sim-arbiter-ok the boot steering / route effects this serve's owner issued (link update / attach / pin / route write order), paired Tier-3 R5-24, R5-34, R5-25
        let effects = driver.io.steering_effects();
        let boot = driver.serve.as_ref().map(|serve| &serve.boot);
        Ok(Self {
            phase: boot_phase(boot, &effects).as_str().to_owned(),
            link: link_of(steering.link).to_owned(),
            prog: steering
                .program
                .map_or_else(|| "none".to_owned(), |program| program.label().to_owned()),
            route: route_of(steering.route).to_owned(),
        })
    }
}

/// `phase`: `start` while `serve` is down or has not called
/// `converge_shared`; `open` once it returned `Ok`; while pending, the step
/// after the last `Ok` steering effect of this `serve`'s owner.
fn boot_phase(boot: Option<&BootCall>, effects: &[SimSteeringEffect]) -> Phase {
    match boot {
        None | Some(BootCall::NotCalled) => Phase::Start,
        Some(BootCall::Opened) => Phase::Open,
        Some(BootCall::Pending(_)) => match effects.last() {
            None => Phase::Load,
            Some(SimSteeringEffect::Load { ok: true, .. }) => Phase::Swap,
            Some(SimSteeringEffect::Attach { adopted: false, ok: true }) => Phase::Pin,
            Some(
                SimSteeringEffect::Attach { adopted: true, ok: true }
                | SimSteeringEffect::Pin { ok: true },
            ) => Phase::Route,
            Some(SimSteeringEffect::Route { ok: true }) => Phase::Open,
            Some(
                SimSteeringEffect::Load { ok: false, .. }
                | SimSteeringEffect::Attach { ok: false, .. }
                | SimSteeringEffect::Pin { ok: false }
                | SimSteeringEffect::Route { ok: false }
                | SimSteeringEffect::Steer { .. },
            ) => Phase::Start,
        },
    }
}

/// `link`: none | attached | pinned.
const fn link_of(link: SimSteeringLink) -> &'static str {
    match link {
        SimSteeringLink::None => "none",
        SimSteeringLink::Attached => "attached",
        SimSteeringLink::Pinned => "pinned",
    }
}

/// `route`: absent | local.
const fn route_of(route: SimPrefixRoute) -> &'static str {
    match route {
        SimPrefixRoute::Absent => "absent",
        SimPrefixRoute::Local => "local",
    }
}

/// Replays `boot_ok` traces against the production owner.
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/prefix_boot.qnt", main = "boot_ok")]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
fn prefix_boot_conformance_boot_ok() -> impl Driver {
    PrefixBootDriver::new()
}

/// Replays `boot_faulty` traces against the production owner.
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/prefix_boot.qnt", main = "boot_faulty")]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
fn prefix_boot_conformance_boot_faulty() -> impl Driver {
    PrefixBootDriver::new()
}

/// Replays `boot_live` traces against the production owner.
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/prefix_boot.qnt", main = "boot_live")]
#[ignore = "pending DELIVER vsock attachment cut (new-step 13, steering load and swap, shared local route)"]
fn prefix_boot_conformance_boot_live() -> impl Driver {
    PrefixBootDriver::new()
}
