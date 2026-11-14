//! Quint conformance driver: `OwnerCore` against the shared-guest owner
//! protocol, conformance main `guest_conf`.
//!
//! Replays `specs/quint/shared-guest-owner/shared_guest.qnt` (referenced in
//! place) on its recording main `guest_conf` (`ENV = false`: the owner is
//! already started and no environment action is enabled) against the
//! production owner core. The step record is the nondeterministic pick of
//! `guest_conf::shared_guest::conf_step`.
//!
//! | Action | Production call |
//! |---|---|
//! | `Init` | a fresh `OwnerCore::new()` |
//! | `Attached{c}` | `handle(Event::Attached(AttachId(c)))` |
//! | `ClientGone{c}` | `handle(Event::ClientGone(AttachId(c)))` |
//! | `BootFinished{ok}` | `handle(Event::BootFinished(ok))` |
//! | `ExecFinished{pass}` | `handle(Event::ExecFinished(in_flight, 0 or 1))` |
//! | `GuestLost` | `handle(Event::GuestLost)` |
//! | `ExecBound` | `handle(Event::ExecBoundElapsed(in_flight))` |
//! | `IdleElapsed` | `handle(Event::IdleElapsed)` |
//!
//! Projection (every variable through the pinned `OwnerCore::observe()`):
//! `guest` <- `OwnerObservation::guest` (NoGuest 0, Booting 1, Ready 2);
//! `gen` <- `generation`; `inflight` <- `in_flight` (0 when none); `queue` <-
//! `queue`; `owners` <- 1 unless `stopped`. Model-only: `lock`, `bad`,
//! `execgen`, `execs`, `faults`, `idleLoss`, `conf_step` and the per-client
//! phases.
//!
//! The environment actions `Arrive`, `ElectBegin`, `OwnerStarted` and
//! `OwnerCrash` are outside conformance replay under the approved exemption
//! (design decision N-4); each invariant they touch is proved by a Tier-3
//! scenario named in `distill/test-scenarios.md`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::collections::BTreeMap;

use itf::Value;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::{AttachId, Event, GuestView, OwnerCore};

const MAIN: &str = "guest_conf";
const MODULE: &str = "shared_guest";

/// The compared variables of one trace state.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct OwnerState {
    guest: i64,
    generation: i64,
    in_flight: i64,
    queue: Vec<i64>,
    owners: i64,
}

fn variables(state: &Value) -> std::result::Result<BTreeMap<String, &Value>, String> {
    let Value::Record(record) = state else {
        return Err(format!("expected a record of variables, found {state:?}"));
    };
    let prefix = format!("{MAIN}::{MODULE}::");
    Ok(record
        .iter()
        .filter_map(|(key, value)| key.strip_prefix(&prefix).map(|name| (name.to_owned(), value)))
        .collect())
}

fn int(value: &Value) -> std::result::Result<i64, String> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::BigInt(big) => i64::try_from(big.get().clone()).map_err(|e| e.to_string()),
        other => Err(format!("expected an int, found {other:?}")),
    }
}

impl OwnerState {
    fn from_value(state: &Value) -> std::result::Result<Self, String> {
        let vars = variables(state)?;
        let var = |name: &str| {
            vars.get(name).copied().ok_or_else(|| format!("{MAIN}::{MODULE}::{name} is missing"))
        };
        let queue = match var("queue")? {
            Value::List(list) => {
                list.iter().map(int).collect::<std::result::Result<Vec<_>, _>>()?
            }
            other => return Err(format!("expected a list, found {other:?}")),
        };
        Ok(Self {
            guest: int(var("guest")?)?,
            generation: int(var("gen")?)?,
            in_flight: int(var("inflight")?)?,
            queue,
            owners: int(var("owners")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for OwnerState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<OwnerDriver> for OwnerState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value).map_err(anyhow_message)?)
    }

    fn from_driver(driver: &OwnerDriver) -> Result<Self> {
        let observed = driver.core.observe();
        Ok(Self {
            guest: match observed.guest {
                GuestView::NoGuest => 0,
                GuestView::Booting => 1,
                GuestView::Ready => 2,
            },
            generation: i64::from(observed.generation),
            in_flight: observed.in_flight.map_or(0, |id| i64::from(id_number(id))),
            queue: observed.queue.into_iter().map(|id| i64::from(id_number(id))).collect(),
            owners: i64::from(!observed.stopped),
        })
    }
}

fn anyhow_message(message: String) -> std::io::Error {
    std::io::Error::other(message)
}

/// The model's client number of an `AttachId`: the model has the clients 1, 2
/// and 3, and the driver builds each `AttachId` from its number.
fn id_number(id: AttachId) -> u32 {
    (1..=3_u32)
        .find(|n| AttachId::new(*n) == id)
        .expect("every attach id of a model trace is one of the model's clients")
}

/// The driver: it holds the production owner core and nothing else.
pub(super) struct OwnerDriver {
    core: OwnerCore,
}

impl OwnerDriver {
    fn new() -> Self {
        Self { core: OwnerCore::new() }
    }

    fn client(c: i64) -> AttachId {
        AttachId::new(u32::try_from(c).expect("a model client number is a small positive int"))
    }

    fn in_flight(&self) -> AttachId {
        self.core
            .observe()
            .in_flight
            .expect("the model enables this action only with an exec in flight")
    }

    fn apply(&mut self, event: Event) -> Result {
        let _effects = self.core.handle(event);
        Ok(())
    }
}

impl Driver for OwnerDriver {
    type State = OwnerState;

    fn config() -> Config {
        Config { state: &[], nondet: &["guest_conf::shared_guest::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            Init => { self.core = OwnerCore::new(); },
            Attached(c: i64) => self.apply(Event::Attached(Self::client(c)))?,
            ClientGone(c: i64) => self.apply(Event::ClientGone(Self::client(c)))?,
            BootFinished(ok: bool) => self.apply(Event::BootFinished(ok))?,
            ExecFinished(pass: bool) => {
                let id = self.in_flight();
                self.apply(Event::ExecFinished(id, i32::from(!pass)))?;
            },
            GuestLost => self.apply(Event::GuestLost)?,
            ExecBound => {
                let id = self.in_flight();
                self.apply(Event::ExecBoundElapsed(id))?;
            },
            IdleElapsed => self.apply(Event::IdleElapsed)?,
        })
    }
}

// ----- named runs of `guest_conf` and randomized traces ---------------------

/// Named run `happy_path_three_clients_share_one_guest`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "happy_path_three_clients_share_one_guest"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_happy_path_three_clients_share_one_guest() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `failure_then_replacement_guest`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "failure_then_replacement_guest"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_failure_then_replacement_guest() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `guest_lost_mid_exec_then_replacement`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "guest_lost_mid_exec_then_replacement"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_guest_lost_mid_exec_then_replacement() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `disconnect_while_running_next_waits`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "disconnect_while_running_next_waits"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_disconnect_while_running_next_waits() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `disconnect_while_queued_is_removed`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "disconnect_while_queued_is_removed"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_disconnect_while_queued_is_removed() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `boot_failure_then_retry_boot_on_next_attach`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "boot_failure_then_retry_boot_on_next_attach"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_boot_failure_then_retry_boot_on_next_attach() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `exec_bound_discards_guest_and_replies_bound`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "exec_bound_discards_guest_and_replies_bound"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_exec_bound_discards_guest_and_replies_bound() -> impl Driver {
    OwnerDriver::new()
}

/// Named run `idle_exit_only_when_empty`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 1,
    test = "idle_exit_only_when_empty"
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_idle_exit_only_when_empty() -> impl Driver {
    OwnerDriver::new()
}

/// Randomized `guest_conf` traces; the seed is printed on failure.
#[quint_run(
    spec = "../../specs/quint/shared-guest-owner/shared_guest.qnt",
    main = "guest_conf",
    max_samples = 64
)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn quint_shared_guest_randomized_traces() -> impl Driver {
    OwnerDriver::new()
}
